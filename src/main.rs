// justgui -- a native GUI shell around `just`, built with Slint.
// Reads a justfile via `just --dump --dump-format json`, renders one
// button per recipe (with input fields for its parameters), streams the
// recipe's output live, and offers a plain-text editor for the justfile.
mod envfile;
mod just_client;
mod layout;
mod process;
mod termout;
mod theme;

use just_client::{build_run_command, load_justfile, JustModel};
use process::Process;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use theme::ThemeConfig;

slint::include_modules!();

/// One running (or finished-but-not-yet-dismissed) recipe invocation.
/// Several can coexist so a long-lived recipe (e.g. one that starts an
/// AppImage and stays up) doesn't block running anything else.
struct Session {
    id: i32,
    recipe_name: String,
    proc: Process,
    log: String,
    finished: bool, // true once its "[exit code N]" line has been appended
}

struct AppState {
    dir: String,
    model: JustModel,
    param_values: Vec<Vec<String>>, // per recipe, per param
    sessions: Vec<Session>,
    next_session_id: i32,
    active_session: i32, // id of the session shown in the output panel; -1 = none
    // The Slint-side session-tab model, mutated in place (see
    // `sync_sessions_model`) rather than replaced wholesale on every poll
    // tick -- replacing a `ModelRc` outright forces Slint's `for` repeater
    // to tear down and rebuild every tab item, which (since `poll_sessions`
    // runs every 50ms) can happen mid-click and silently swallow the click.
    sessions_model: Rc<VecModel<RunSession>>,
    // Same in-place-mutation reasoning as `sessions_model`: recipe tiles now
    // reflect live running/needs-input state via `refresh_recipe_running_state`,
    // called from the 50ms poll tick, so these can't be rebuilt wholesale
    // either without risking the same click-swallowing bug.
    recipes_model: Rc<VecModel<RecipeData>>,
    param_recipes_model: Rc<VecModel<RecipeData>>,
    edit_dirty: bool,
    edit_status: String,
    theme: ThemeConfig,
    editing_theme: bool, // true while a live (unsaved) theme edit is in effect
    layout: layout::LayoutConfig,
    last_linted_buffer: String, // edit-buffer text the linter last checked
    edit_lint_error: String,
    env_file: envfile::EnvFile, // Settings popup, backed by the project's .env
}

fn apply_theme(ui: &AppWindow, cfg: &ThemeConfig) {
    let t = ui.global::<Theme>();
    t.set_background(theme::parse_color(&cfg.background));
    t.set_panel_background(theme::parse_color(&cfg.panel_background));
    t.set_border(theme::parse_color(&cfg.border));
    t.set_accent(theme::parse_color(&cfg.accent));
    t.set_text(theme::parse_color(&cfg.text));
    t.set_muted_text(theme::parse_color(&cfg.muted_text));
    t.set_error(theme::parse_color(&cfg.error));
    t.set_warning(theme::parse_color(&cfg.warning));
    t.set_corner_radius(cfg.corner_radius);
    t.set_font_family(cfg.font_family.clone().into());
    t.set_font_size(cfg.font_size);
    t.set_dark_mode(cfg.mode.eq_ignore_ascii_case("dark"));
}

/// Re-resolves the theme for the current directory and re-applies it only
/// if something actually changed -- called both on reload and on a timer,
/// so editing `justgui.toml` restyles the running app with no restart.
/// Skipped while an in-app theme edit is live and unsaved, so this poll
/// doesn't immediately stomp it back to what's on disk.
fn refresh_theme(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    if state.borrow().editing_theme {
        return;
    }
    let dir = state.borrow().dir.clone();
    let resolved = theme::resolve(&dir);
    let changed = resolved != state.borrow().theme;
    if changed {
        state.borrow_mut().theme = resolved.clone();
        apply_theme(ui, &resolved);
    }
}

/// Pushes the current theme's editable numeric/string fields into the
/// Slint side mirrors the theme-editor popup reads from (Theme itself only
/// exposes color/length/bool types, not the plain int/string a SpinBox or
/// LineEdit binds to).
fn sync_theme_ui(ui: &AppWindow, cfg: &ThemeConfig) {
    ui.set_theme_corner_radius(cfg.corner_radius.round() as i32);
    ui.set_theme_font_size(cfg.font_size.round() as i32);
    ui.set_theme_font_family(cfg.font_family.clone().into());
    ui.set_theme_dark_mode(cfg.mode.eq_ignore_ascii_case("dark"));
}

fn param_hint(p: &just_client::JustParam) -> String {
    if p.variadic {
        "(space-separated)".to_string()
    } else if p.has_default {
        format!("[{}]", p.default_value)
    } else {
        "(required)".to_string()
    }
}

fn recipe_data(st: &AppState, idx: usize) -> RecipeData {
    let r = &st.model.recipes[idx];
    let entry = st.layout.recipe.iter().find(|e| e.name == r.name);
    let params: Vec<ParamData> = r
        .params
        .iter()
        .map(|p| ParamData {
            name: p.name.clone().into(),
            hint: param_hint(p).into(),
            default_value: if p.has_default {
                p.default_value.clone().into()
            } else {
                "".into()
            },
        })
        .collect();
    // A running session of the same name -- visual feedback that clicking
    // Run actually did something, and that this particular recipe (not just
    // "something") is the one currently active.
    let running_session = st.sessions.iter().find(|s| s.recipe_name == r.name && !s.finished);
    RecipeData {
        recipe_index: idx as i32,
        name: r.name.clone().into(),
        doc: r.doc.clone().into(),
        is_private: r.is_private,
        is_shown: entry.is_none_or(|e| e.shown),
        is_running: running_session.is_some(),
        needs_input: running_session.is_some_and(|s| termout::looks_like_prompt(&s.log)),
        color: theme::parse_color(entry.map_or(layout::PALETTE[0], |e| e.color.as_str())),
        params: ModelRc::new(VecModel::from(params)),
    }
}

/// Registers `st`'s persistent list models with the UI. Must be called once
/// right after constructing both `ui` and `st` (before the first `reload`)
/// -- everything after this mutates these models in place (see
/// `sync_sessions_model`/`sync_recipe_model`) rather than ever calling
/// `set_recipes`/`set_param_recipes`/`set_sessions` again.
fn bind_models(ui: &AppWindow, st: &AppState) {
    ui.set_recipes(ModelRc::from(st.recipes_model.clone()));
    ui.set_param_recipes(ModelRc::from(st.param_recipes_model.clone()));
    ui.set_sessions(ModelRc::from(st.sessions_model.clone()));
}

fn sync_ui(ui: &AppWindow, st: &AppState) {
    let show_private = ui.get_show_private();

    // Recipes are rendered in `layout` position order, not justfile order --
    // an unconfigured recipe (no layout entry yet, shouldn't normally happen
    // since `reload` calls `layout::sync_entries`) sorts first.
    let mut order: Vec<usize> = (0..st.model.recipes.len()).collect();
    order.sort_by_key(|&i| {
        let name = &st.model.recipes[i].name;
        st.layout.recipe.iter().find(|e| &e.name == name).map_or(0, |e| e.position)
    });

    let all_recipes: Vec<RecipeData> = order.iter().map(|&i| recipe_data(st, i)).collect();
    let visible: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&i| {
            let r = &st.model.recipes[i];
            let shown = st.layout.recipe.iter().find(|e| e.name == r.name).is_none_or(|e| e.shown);
            shown && (!r.is_private || show_private)
        })
        .collect();

    // Parameterless recipes render as compact grid buttons; recipes that
    // take parameters get a full-width card with the fields shown inline
    // instead of hidden behind a popup (see app.slint) -- so a recipe with
    // an optional/blank parameter that prompts interactively on its own
    // (e.g. a shebang recipe using `select`) is easy to just leave blank
    // and Run.
    let recipes: Vec<RecipeData> = visible
        .iter()
        .filter(|&&i| st.model.recipes[i].params.is_empty())
        .map(|&i| recipe_data(st, i))
        .collect();
    let param_recipes: Vec<RecipeData> = visible
        .iter()
        .filter(|&&i| !st.model.recipes[i].params.is_empty())
        .map(|&i| recipe_data(st, i))
        .collect();

    ui.set_all_recipes(ModelRc::new(VecModel::from(all_recipes)));
    sync_recipe_model(&st.recipes_model, &recipes);
    sync_recipe_model(&st.param_recipes_model, &param_recipes);
    ui.set_load_error(st.model.error.clone().into());
    ui.set_justfile_path(st.model.justfile_path.clone().into());
    ui.set_edit_dirty(st.edit_dirty);
    ui.set_edit_status(st.edit_status.clone().into());
    ui.set_edit_lint_error(st.edit_lint_error.clone().into());
    sync_sessions_ui(ui, st);
    ui.set_palette(ModelRc::new(VecModel::from(
        layout::PALETTE
            .iter()
            .map(|c| theme::parse_color(c))
            .collect::<Vec<_>>(),
    )));

    sync_theme_ui(ui, &st.theme);
    sync_settings_ui(ui, st);
}

/// A `.env` value is shown as a toggle if it's exactly (case-insensitively)
/// "true"/"false", and as a plain text field otherwise -- covers strings and
/// numbers alike without guessing at numeric ranges/precision we can't know.
fn setting_data(key: &str, value: &str) -> SettingData {
    let is_bool = value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false");
    SettingData {
        key: key.into(),
        value: value.into(),
        is_bool,
        bool_value: is_bool && value.eq_ignore_ascii_case("true"),
    }
}

fn sync_settings_ui(ui: &AppWindow, st: &AppState) {
    let settings: Vec<SettingData> = st.env_file.vars().map(|(k, v)| setting_data(k, v)).collect();
    ui.set_settings(ModelRc::new(VecModel::from(settings)));
}

/// Updates `model` in place to match `desired`, using targeted
/// insert/remove/`set_row_data` calls instead of `set_vec` -- `set_vec`
/// (and replacing the `ModelRc` wholesale) sends Slint's `for` repeater a
/// "reset" notification that tears down and rebuilds every row, which loses
/// any in-progress interaction (e.g. a click whose press and release land on
/// either side of a rebuild). `set_row_data` instead sends a targeted
/// "this one row changed" notification that leaves untouched rows -- and
/// their `TouchArea`s -- alone. Assumes `desired`'s relative order matches
/// `model`'s for ids present in both, which holds here since sessions are
/// only ever appended or removed, never reordered.
fn sync_sessions_model(model: &VecModel<RunSession>, desired: &[RunSession]) {
    let mut i = 0;
    while i < model.row_count() {
        let keep = model.row_data(i).is_some_and(|row| desired.iter().any(|s| s.id == row.id));
        if keep {
            i += 1;
        } else {
            model.remove(i);
        }
    }
    for (i, want) in desired.iter().enumerate() {
        match model.row_data(i) {
            Some(have) if have.id == want.id => {
                if have.name != want.name || have.running != want.running || have.needs_input != want.needs_input {
                    model.set_row_data(i, want.clone());
                }
            }
            _ => model.insert(i, want.clone()),
        }
    }
}

/// Same in-place-update reasoning as `sync_sessions_model`, applied to a
/// recipe grid/card model -- rows are keyed by the stable `recipe_index`.
fn sync_recipe_model(model: &VecModel<RecipeData>, desired: &[RecipeData]) {
    let mut i = 0;
    while i < model.row_count() {
        let keep = model.row_data(i).is_some_and(|row| desired.iter().any(|d| d.recipe_index == row.recipe_index));
        if keep {
            i += 1;
        } else {
            model.remove(i);
        }
    }
    for (i, want) in desired.iter().enumerate() {
        match model.row_data(i) {
            Some(have) if have.recipe_index == want.recipe_index => model.set_row_data(i, want.clone()),
            _ => model.insert(i, want.clone()),
        }
    }
}

/// Refreshes just the is-running/needs-input flags on the recipe grid/card
/// models to reflect current sessions -- called on the 50ms poll tick, so
/// (like `sync_sessions_model`) it only touches rows whose state actually
/// changed rather than rebuilding anything, to avoid swallowing a click
/// that's mid-gesture on a tile's Run/reorder/color handle.
fn refresh_recipe_running_state(st: &AppState) {
    for model in [&st.recipes_model, &st.param_recipes_model] {
        for i in 0..model.row_count() {
            let Some(row) = model.row_data(i) else { continue };
            let session = st.sessions.iter().find(|s| s.recipe_name == row.name.as_str() && !s.finished);
            let is_running = session.is_some();
            let needs_input = session.is_some_and(|s| termout::looks_like_prompt(&s.log));
            if row.is_running != is_running || row.needs_input != needs_input {
                let mut updated = row;
                updated.is_running = is_running;
                updated.needs_input = needs_input;
                model.set_row_data(i, updated);
            }
        }
    }
}

/// Pushes the session list and the active session's log/running state.
/// Cheap enough to call on every poll tick (unlike the full `sync_ui`,
/// which recomputes the recipe models too).
fn sync_sessions_ui(ui: &AppWindow, st: &AppState) {
    let sessions: Vec<RunSession> = st
        .sessions
        .iter()
        .map(|s| RunSession {
            id: s.id,
            name: s.recipe_name.clone().into(),
            running: !s.finished,
            needs_input: !s.finished && termout::looks_like_prompt(&s.log),
        })
        .collect();
    sync_sessions_model(&st.sessions_model, &sessions);
    ui.set_active_session(st.active_session);

    match st.sessions.iter().find(|s| s.id == st.active_session) {
        Some(active) => {
            ui.set_run_log(active.log.clone().into());
            ui.set_running(!active.finished);
            ui.set_needs_input(!active.finished && termout::looks_like_prompt(&active.log));
        }
        None => {
            ui.set_run_log("".into());
            ui.set_running(false);
            ui.set_needs_input(false);
        }
    }
}

/// Drops any finished session other than the one currently shown in the
/// output panel -- once you're not looking at a finished recipe's output
/// anymore there's no reason to keep its tab around forever. The active
/// session is exempt so its final output/exit code stays visible until you
/// switch away or close it yourself.
fn sweep_finished_sessions(st: &mut AppState) {
    let active = st.active_session;
    st.sessions.retain(|s| !s.finished || s.id == active);
}

fn reload(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    st.model = load_justfile(&st.dir);
    st.layout = layout::load(&st.dir);
    let AppState { layout, model, .. } = &mut *st;
    layout::sync_entries(layout, &model.recipes);
    let _ = layout::save(&st.dir, &st.layout);
    st.editing_theme = false;
    st.env_file = envfile::EnvFile::load(&st.dir);

    st.param_values = st
        .model
        .recipes
        .iter()
        .map(|r| {
            r.params
                .iter()
                .map(|p| {
                    if p.has_default {
                        p.default_value.clone()
                    } else {
                        String::new()
                    }
                })
                .collect()
        })
        .collect();

    st.edit_dirty = false;
    st.edit_status.clear();

    let mut edit_buffer = String::new();
    if !st.model.justfile_path.is_empty() {
        if let Ok(text) = std::fs::read_to_string(&st.model.justfile_path) {
            edit_buffer = text;
        }
    }

    sync_ui(ui, &st);
    ui.set_edit_buffer(edit_buffer.into());
    drop(st);

    refresh_theme(ui, state);
}

/// Always starts a brand-new session rather than refusing when something
/// else is already running -- recipes that need to stay running (e.g. one
/// that starts an AppImage and stays up) shouldn't block anything else.
fn run_recipe(ui: &AppWindow, state: &Rc<RefCell<AppState>>, idx: usize) {
    let mut st = state.borrow_mut();
    let Some(recipe) = st.model.recipes.get(idx).cloned() else {
        return;
    };
    let Some(param_values) = st.param_values.get(idx).cloned() else {
        return;
    };

    let cmd = build_run_command(&recipe, &param_values);
    let dir = st.dir.clone();
    let mut proc = Process::new();
    proc.start(&cmd, &dir);

    let id = st.next_session_id;
    st.next_session_id += 1;
    st.sessions.push(Session {
        id,
        recipe_name: recipe.name.clone(),
        proc,
        log: format!("$ {cmd}\n"),
        finished: false,
    });
    st.active_session = id;

    sweep_finished_sessions(&mut st);
    sync_sessions_ui(ui, &st);
    refresh_recipe_running_state(&st);
}

fn poll_sessions(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    if st.sessions.is_empty() {
        return;
    }

    for session in st.sessions.iter_mut() {
        if session.finished {
            continue;
        }
        let (chunk, done) = session.proc.poll();
        if !chunk.is_empty() {
            termout::append_chunk(&mut session.log, &chunk);
        }
        if done {
            let code = session.proc.exit_code();
            session.log.push_str(&format!("\n[exit code {code}]\n"));
            session.finished = true;
        }
    }

    sweep_finished_sessions(&mut st);
    sync_sessions_ui(ui, &st);
    refresh_recipe_running_state(&st);
}

fn select_session(ui: &AppWindow, state: &Rc<RefCell<AppState>>, id: i32) {
    let mut st = state.borrow_mut();
    st.active_session = id;
    sweep_finished_sessions(&mut st);
    sync_sessions_ui(ui, &st);
}

/// Stops tracking/showing a session. This does not kill the underlying
/// process if it's still running -- there's still no kill capability --
/// its reader thread just keeps going, detached, until the child exits on
/// its own (same as always happened when a new run replaced a previous
/// `Process` in place, before sessions existed).
fn close_session(ui: &AppWindow, state: &Rc<RefCell<AppState>>, id: i32) {
    let mut st = state.borrow_mut();
    st.sessions.retain(|s| s.id != id);
    if st.active_session == id {
        st.active_session = st.sessions.last().map(|s| s.id).unwrap_or(-1);
    }
    sweep_finished_sessions(&mut st);
    sync_sessions_ui(ui, &st);
    refresh_recipe_running_state(&st);
}

fn save_edit(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let path = state.borrow().model.justfile_path.clone();
    if path.is_empty() {
        let mut st = state.borrow_mut();
        st.edit_status = "no justfile path known; cannot save".to_string();
        ui.set_edit_status(st.edit_status.clone().into());
        return;
    }

    let buffer = ui.get_edit_buffer().to_string();
    if std::fs::write(&path, &buffer).is_err() {
        let mut st = state.borrow_mut();
        st.edit_status = format!("failed to write {path}");
        ui.set_edit_status(st.edit_status.clone().into());
        return;
    }

    reload(ui, state);

    let mut st = state.borrow_mut();
    st.edit_status = if st.model.error.is_empty() {
        "saved".to_string()
    } else {
        "saved, but justfile now fails to parse".to_string()
    };
    st.edit_dirty = false;
    let status = st.edit_status.clone();
    drop(st);

    ui.set_edit_buffer(buffer.into());
    ui.set_edit_dirty(false);
    ui.set_edit_status(status.into());
}

/// Re-lints the edit buffer (the *unsaved* text, not what's on disk) if
/// it's changed since the last check -- called on a timer, so this is the
/// short-circuit that keeps it from re-invoking `just` on every tick while
/// the user isn't actively typing.
fn lint_edit_buffer(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let buffer = ui.get_edit_buffer().to_string();
    let dir = {
        let st = state.borrow();
        if buffer == st.last_linted_buffer {
            return;
        }
        st.dir.clone()
    };
    let error = just_client::lint_justfile(&buffer, &dir);
    let mut st = state.borrow_mut();
    st.last_linted_buffer = buffer;
    st.edit_lint_error = error;
    ui.set_edit_lint_error(st.edit_lint_error.clone().into());
}

/// Opens the justfile in $VISUAL/$EDITOR inside a separate terminal window
/// (same editor-resolution convention `just --edit` uses). Doesn't try to
/// embed the editor's output in our own window -- a TUI editor needs a
/// real terminal, which this just hands off to entirely, so there's no
/// rendering-fidelity concern the way there would be for a recipe's
/// output. Best-effort: if no terminal emulator is found, this silently
/// no-ops rather than failing loudly over what's a convenience.
fn edit_externally(state: &Rc<RefCell<AppState>>) {
    let path = state.borrow().model.justfile_path.clone();
    if path.is_empty() {
        return;
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    if cfg!(windows) {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "cmd", "/K", &editor, &path])
            .spawn();
        return;
    }

    const TERMINALS: &[(&str, &str)] = &[
        ("x-terminal-emulator", "-e"),
        ("gnome-terminal", "--"),
        ("konsole", "-e"),
        ("xterm", "-e"),
    ];
    for (term, flag) in TERMINALS {
        if std::process::Command::new(term).arg(flag).arg(&editor).arg(&path).spawn().is_ok() {
            return;
        }
    }
}

/// Sends a line to the active session's pty stdin. The pty's own line
/// discipline echoes it back through the normal output stream (like a real
/// terminal would), so it shows up in the log via the next `poll_sessions`
/// -- no need to echo it here ourselves.
fn send_input(state: &Rc<RefCell<AppState>>, text: String) {
    let st = state.borrow();
    let active = st.active_session;
    if let Some(session) = st.sessions.iter().find(|s| s.id == active) {
        if !session.finished {
            session.proc.send_input(&text);
        }
    }
}

fn close_input(state: &Rc<RefCell<AppState>>) {
    let st = state.borrow();
    let active = st.active_session;
    if let Some(session) = st.sessions.iter().find(|s| s.id == active) {
        session.proc.close_input();
    }
}

fn clear_log(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    let active = st.active_session;
    if let Some(session) = st.sessions.iter_mut().find(|s| s.id == active) {
        session.log.clear();
    }
    ui.set_run_log("".into());
}

fn set_setting_value(ui: &AppWindow, state: &Rc<RefCell<AppState>>, key: &str, value: &str) {
    let mut st = state.borrow_mut();
    st.env_file.set(key, value);
    let _ = st.env_file.save(&st.dir);
    sync_settings_ui(ui, &st);
}

/// Adds a new blank (empty-string, so it starts out as a text field) `.env`
/// entry. A no-op for an empty or already-existing key -- silently, since
/// this is driven by a plain text field next to an "Add" button rather than
/// a form with its own validation/error display.
fn add_setting(ui: &AppWindow, state: &Rc<RefCell<AppState>>, key: &str) {
    let key = key.trim();
    if key.is_empty() {
        return;
    }
    let mut st = state.borrow_mut();
    if st.env_file.vars().any(|(k, _)| k == key) {
        return;
    }
    st.env_file.set(key, "");
    let _ = st.env_file.save(&st.dir);
    sync_settings_ui(ui, &st);
}

fn remove_setting(ui: &AppWindow, state: &Rc<RefCell<AppState>>, key: &str) {
    let mut st = state.borrow_mut();
    st.env_file.remove(key);
    let _ = st.env_file.save(&st.dir);
    sync_settings_ui(ui, &st);
}

fn save_layout(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let st = state.borrow();
    let _ = layout::save(&st.dir, &st.layout);
    sync_ui(ui, &st);
    drop(st);
}

fn toggle_shown(ui: &AppWindow, state: &Rc<RefCell<AppState>>, ri: usize, shown: bool) {
    {
        let mut st = state.borrow_mut();
        let Some(name) = st.model.recipes.get(ri).map(|r| r.name.clone()) else {
            return;
        };
        if let Some(entry) = st.layout.recipe.iter_mut().find(|e| e.name == name) {
            entry.shown = shown;
        }
    }
    save_layout(ui, state);
}

/// Swaps display position between the recipe at `dragged_idx` and the one
/// at `target_idx` (both are `RecipeData.recipe-index` values, i.e. indices
/// into `st.model.recipes`).
fn recipe_dropped(ui: &AppWindow, state: &Rc<RefCell<AppState>>, dragged_idx: usize, target_idx: usize) {
    {
        let mut st = state.borrow_mut();
        let Some(dragged_name) = st.model.recipes.get(dragged_idx).map(|r| r.name.clone()) else {
            return;
        };
        let Some(target_name) = st.model.recipes.get(target_idx).map(|r| r.name.clone()) else {
            return;
        };
        let Some(dragged_pos) = st.layout.recipe.iter().position(|e| e.name == dragged_name) else {
            return;
        };
        let Some(target_pos) = st.layout.recipe.iter().position(|e| e.name == target_name) else {
            return;
        };
        let dragged_position = st.layout.recipe[dragged_pos].position;
        let target_position = st.layout.recipe[target_pos].position;
        st.layout.recipe[dragged_pos].position = target_position;
        st.layout.recipe[target_pos].position = dragged_position;
    }
    save_layout(ui, state);
}

fn set_recipe_color(ui: &AppWindow, state: &Rc<RefCell<AppState>>, recipe_idx: usize, color: slint::Color) {
    {
        let mut st = state.borrow_mut();
        let Some(name) = st.model.recipes.get(recipe_idx).map(|r| r.name.clone()) else {
            return;
        };
        let hex = theme::color_to_hex(&color);
        if let Some(entry) = st.layout.recipe.iter_mut().find(|e| e.name == name) {
            entry.color = hex;
        }
    }
    save_layout(ui, state);
}

fn set_theme_color(ui: &AppWindow, state: &Rc<RefCell<AppState>>, field: &str, color: slint::Color) {
    let mut st = state.borrow_mut();
    let hex = theme::color_to_hex(&color);
    let applied = match field {
        "background" => &mut st.theme.background,
        "panel-background" => &mut st.theme.panel_background,
        "border" => &mut st.theme.border,
        "accent" => &mut st.theme.accent,
        "text" => &mut st.theme.text,
        "muted-text" => &mut st.theme.muted_text,
        "error" => &mut st.theme.error,
        "warning" => &mut st.theme.warning,
        _ => return,
    };
    *applied = hex;
    st.editing_theme = true;
    apply_theme(ui, &st.theme);
}

fn set_theme_field(ui: &AppWindow, state: &Rc<RefCell<AppState>>, field: &str, value: &str) {
    let mut st = state.borrow_mut();
    match field {
        "mode" => st.theme.mode = value.to_string(),
        "corner-radius" => {
            if let Ok(v) = value.parse::<f32>() {
                st.theme.corner_radius = v;
            }
        }
        "font-size" => {
            if let Ok(v) = value.parse::<f32>() {
                st.theme.font_size = v;
            }
        }
        "font-family" => st.theme.font_family = value.to_string(),
        _ => return,
    }
    st.editing_theme = true;
    apply_theme(ui, &st.theme);
}

fn save_theme(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    let status = match theme::save(&st.dir, &st.theme) {
        Ok(()) => "saved".to_string(),
        Err(e) => format!("failed to save: {e}"),
    };
    st.editing_theme = false;
    ui.set_theme_status(status.into());
}

// Slint's real (winit) backend is thread-affine and can only be installed
// once per process, but `cargo test` runs each test on its own thread by
// default -- so any test that calls `AppWindow::new()` needs a platform
// that doesn't care which thread it's on. `MinimalSoftwareWindow` is
// exactly that (no real display/event loop needed), and installing it per
// *thread* via `thread_local!` (rather than once process-wide) sidesteps
// cross-test interference entirely: each test thread gets its own
// independent fake window. This mirrors the pattern Slint's own test
// suite uses (`tests/common/mod.rs` in the `slint` crate).
#[cfg(test)]
mod test_support {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use slint::PhysicalSize;
    use std::rc::Rc;

    thread_local! {
        static WINDOW: Rc<MinimalSoftwareWindow> =
            MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    }

    struct TestPlatform;
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(WINDOW.with(|w| w.clone()))
        }
    }

    /// Installs the software-rendering test platform for the current
    /// thread (idempotent -- `.ok()` swallows the "already set" error a
    /// second call on the same thread would otherwise return) and returns
    /// its window, sized to the app's normal default.
    pub fn setup() -> Rc<MinimalSoftwareWindow> {
        slint::platform::set_platform(Box::new(TestPlatform)).ok();
        let window = WINDOW.with(|w| w.clone());
        window.set_size(PhysicalSize::new(1000, 700));
        window
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use slint::Model;

    /// Drives the real `AppWindow` + callback pipeline exactly as a click
    /// would (via `invoke_*`, no pixel/mouse simulation needed), against a
    /// scratch justfile with a shebang recipe whose own script prompts
    /// interactively (a bash `select` menu) when its parameter is left
    /// blank -- the scenario an inline, always-visible parameter field is
    /// meant to support (leave it blank, click Run, respond to the
    /// recipe's own prompt through the input box).
    #[test]
    fn recipe_with_blank_param_reaches_its_own_interactive_prompt() {
        test_support::setup();
        let dir = std::env::temp_dir().join(format!("justgui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("justfile"),
            r#"jam PROFILE="":
    #!/usr/bin/env bash
    set -e
    profile="{{PROFILE}}"
    if [ -z "$profile" ]; then
        echo "Select a network emulation profile:"
        PS3="profile> "
        select choice in none fast slow flaky offline; do
            if [ -n "$choice" ]; then profile="$choice"; break; fi
            echo "Invalid selection."
        done
    fi
    echo "chosen profile: $profile"
"#,
        )
        .unwrap();
        let dir = dir.to_string_lossy().into_owned();

        let ui = AppWindow::new().expect("failed to create Slint window");
        let state = Rc::new(RefCell::new(AppState {
            dir: dir.clone(),
            model: JustModel::default(),
            param_values: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
            active_session: -1,
            sessions_model: Rc::new(VecModel::default()),
            recipes_model: Rc::new(VecModel::default()),
            param_recipes_model: Rc::new(VecModel::default()),
            edit_dirty: false,
            edit_status: String::new(),
            theme: ThemeConfig::default(),
            editing_theme: false,
            layout: layout::LayoutConfig::default(),
            last_linted_buffer: String::new(),
            edit_lint_error: String::new(),
            env_file: envfile::EnvFile::default(),
        }));
        bind_models(&ui, &state.borrow());

        ui.set_directory(dir.into());
        reload(&ui, &state);

        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_run_recipe(move |idx| {
                if let Some(ui) = ui_handle.upgrade() {
                    run_recipe(&ui, &state, idx as usize);
                }
            });
        }
        {
            let state = state.clone();
            ui.on_send_input(move |text| {
                send_input(&state, text.to_string());
            });
        }

        // `jam` has a parameter, so it should be in `param-recipes` (the
        // inline-fields card list), not the plain `recipes` grid.
        assert!(ui.get_recipes().iter().all(|r| r.name != "jam"));
        let idx = ui
            .get_param_recipes()
            .iter()
            .find(|r| r.name == "jam")
            .expect("jam recipe should be in param-recipes")
            .recipe_index;

        // Leave PROFILE at its blank default and run -- this is what a
        // user leaving the inline field empty and clicking Run does.
        ui.invoke_run_recipe(idx);

        let mut saw_prompt = false;
        for _ in 0..50 {
            poll_sessions(&ui, &state);
            let log = ui.get_run_log().to_string();
            if log.contains("profile>") && !saw_prompt {
                saw_prompt = true;
                ui.invoke_send_input("2".into());
            }
            if log.contains("chosen profile") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let final_log = ui.get_run_log().to_string();
        assert!(saw_prompt, "never saw the select prompt in run_log: {final_log:?}");
        assert!(final_log.contains("chosen profile: fast"), "final log: {final_log:?}");

        let _ = std::fs::remove_dir_all(&state.borrow().dir);
    }

    /// Starting a second recipe while a long-lived one is still running
    /// must not be blocked -- this is the whole point of sessions (the
    /// user's example: a recipe that starts an AppImage and stays up).
    #[test]
    fn a_long_running_recipe_does_not_block_starting_another() {
        test_support::setup();
        let dir = std::env::temp_dir().join(format!("justgui-sessions-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("justfile"),
            "stay-up:\n    sleep 5\n\nquick:\n    echo quick done\n",
        )
        .unwrap();
        let dir = dir.to_string_lossy().into_owned();

        let ui = AppWindow::new().expect("failed to create Slint window");
        let state = Rc::new(RefCell::new(AppState {
            dir: dir.clone(),
            model: JustModel::default(),
            param_values: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
            active_session: -1,
            sessions_model: Rc::new(VecModel::default()),
            recipes_model: Rc::new(VecModel::default()),
            param_recipes_model: Rc::new(VecModel::default()),
            edit_dirty: false,
            edit_status: String::new(),
            theme: ThemeConfig::default(),
            editing_theme: false,
            layout: layout::LayoutConfig::default(),
            last_linted_buffer: String::new(),
            edit_lint_error: String::new(),
            env_file: envfile::EnvFile::default(),
        }));
        bind_models(&ui, &state.borrow());
        ui.set_directory(dir.clone().into());
        reload(&ui, &state);
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_run_recipe(move |idx| {
                if let Some(ui) = ui_handle.upgrade() {
                    run_recipe(&ui, &state, idx as usize);
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_select_session(move |id| {
                if let Some(ui) = ui_handle.upgrade() {
                    select_session(&ui, &state, id);
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_close_session(move |id| {
                if let Some(ui) = ui_handle.upgrade() {
                    close_session(&ui, &state, id);
                }
            });
        }

        let stay_up_idx = state.borrow().model.recipes.iter().position(|r| r.name == "stay-up").unwrap();
        let quick_idx = state.borrow().model.recipes.iter().position(|r| r.name == "quick").unwrap();

        ui.invoke_run_recipe(stay_up_idx as i32);
        assert_eq!(state.borrow().sessions.len(), 1);
        assert!(state.borrow().sessions[0].proc.running());

        // Starting a second recipe while the first is still running must
        // succeed, not be silently dropped.
        ui.invoke_run_recipe(quick_idx as i32);
        assert_eq!(state.borrow().sessions.len(), 2, "second recipe should have started its own session");

        let quick_id = state.borrow().sessions[1].id;
        assert_eq!(ui.get_active_session(), quick_id, "starting a session should make it active");

        for _ in 0..50 {
            poll_sessions(&ui, &state);
            if ui.get_run_log().contains("quick done") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ui.get_run_log().contains("quick done"), "log: {:?}", ui.get_run_log());

        // The long-lived one should still be running, undisturbed.
        assert!(state.borrow().sessions.iter().find(|s| s.recipe_name == "stay-up").unwrap().proc.running());

        // Switching back to it should show its own log, not the quick one's.
        let stay_up_id = state.borrow().sessions[0].id;
        ui.invoke_select_session(stay_up_id);
        assert!(!ui.get_run_log().contains("quick done"));
        assert_eq!(ui.get_active_session(), stay_up_id);

        // The finished "quick" session is no longer the active one, so it
        // should have been swept away automatically -- no need to keep a
        // finished recipe's tab around once you're not looking at it.
        assert_eq!(state.borrow().sessions.len(), 1, "finished, inactive session should auto-close");
        assert_eq!(state.borrow().sessions[0].id, stay_up_id);

        // Closing a still-running session should just stop tracking it,
        // not hang or panic.
        ui.invoke_close_session(stay_up_id);
        assert_eq!(state.borrow().sessions.len(), 0);

        let _ = std::fs::remove_dir_all(&state.borrow().dir);
    }

    /// End-to-end check of the Settings popup's `.env` round trip, through
    /// the same callback pipeline a real click would use: load picks up an
    /// existing `.env`, editing/toggling/adding/removing all persist to
    /// disk and update `ui.get_settings()` in step.
    #[test]
    fn settings_popup_edits_round_trip_through_env_file() {
        test_support::setup();
        let dir = std::env::temp_dir().join(format!("justgui-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("justfile"), "build:\n    echo hi\n").unwrap();
        std::fs::write(dir.join(".env"), "FOO=bar\nDEBUG=true\n").unwrap();
        let dir = dir.to_string_lossy().into_owned();

        let ui = AppWindow::new().expect("failed to create Slint window");
        let state = Rc::new(RefCell::new(AppState {
            dir: dir.clone(),
            model: JustModel::default(),
            param_values: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
            active_session: -1,
            sessions_model: Rc::new(VecModel::default()),
            recipes_model: Rc::new(VecModel::default()),
            param_recipes_model: Rc::new(VecModel::default()),
            edit_dirty: false,
            edit_status: String::new(),
            theme: ThemeConfig::default(),
            editing_theme: false,
            layout: layout::LayoutConfig::default(),
            last_linted_buffer: String::new(),
            edit_lint_error: String::new(),
            env_file: envfile::EnvFile::default(),
        }));
        bind_models(&ui, &state.borrow());
        ui.set_directory(dir.clone().into());
        reload(&ui, &state);
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_setting_value_edited(move |key, value| {
                if let Some(ui) = ui_handle.upgrade() {
                    set_setting_value(&ui, &state, key.as_str(), value.as_str());
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_setting_bool_toggled(move |key, value| {
                if let Some(ui) = ui_handle.upgrade() {
                    set_setting_value(&ui, &state, key.as_str(), if value { "true" } else { "false" });
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_add_setting(move |key| {
                if let Some(ui) = ui_handle.upgrade() {
                    add_setting(&ui, &state, key.as_str());
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_remove_setting(move |key| {
                if let Some(ui) = ui_handle.upgrade() {
                    remove_setting(&ui, &state, key.as_str());
                }
            });
        }

        // Loaded correctly, with the right widget kind inferred per value.
        let settings = ui.get_settings();
        let foo = settings.iter().find(|s| s.key == "FOO").expect("FOO should be loaded");
        assert!(!foo.is_bool);
        assert_eq!(foo.value, "bar");
        let debug = settings.iter().find(|s| s.key == "DEBUG").expect("DEBUG should be loaded");
        assert!(debug.is_bool);
        assert!(debug.bool_value);

        // Toggling a bool persists to disk and updates the model.
        ui.invoke_setting_bool_toggled("DEBUG".into(), false);
        assert!(!ui.get_settings().iter().find(|s| s.key == "DEBUG").unwrap().bool_value);
        assert_eq!(envfile::EnvFile::load(&dir).vars().find(|(k, _)| *k == "DEBUG").unwrap().1, "false");

        // Editing a text value persists too.
        ui.invoke_setting_value_edited("FOO".into(), "baz".into());
        assert_eq!(ui.get_settings().iter().find(|s| s.key == "FOO").unwrap().value, "baz");
        assert_eq!(envfile::EnvFile::load(&dir).vars().find(|(k, _)| *k == "FOO").unwrap().1, "baz");

        // Adding a new key shows up as a (non-bool) setting.
        ui.invoke_add_setting("NEW_KEY".into());
        assert!(ui.get_settings().iter().any(|s| s.key == "NEW_KEY"));

        // Removing drops it from both the model and disk.
        ui.invoke_remove_setting("FOO".into());
        assert!(!ui.get_settings().iter().any(|s| s.key == "FOO"));
        assert!(!envfile::EnvFile::load(&dir).vars().any(|(k, _)| k == "FOO"));

        let _ = std::fs::remove_dir_all(&state.borrow().dir);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use slint::platform::software_renderer::MinimalSoftwareWindow;
    use slint::{PhysicalSize, Rgb8Pixel};
    use std::rc::Rc;

    /// Renders `ui` (already constructed/populated) into an RGB8 buffer at
    /// `width`x`height` via Slint's software renderer -- no real display or
    /// window manager needed, just a `MinimalSoftwareWindow` test platform.
    fn render(window: &Rc<MinimalSoftwareWindow>, width: u32, height: u32) -> Vec<Rgb8Pixel> {
        window.set_size(PhysicalSize::new(width, height));
        let mut buffer = vec![Rgb8Pixel::new(0, 0, 0); (width * height) as usize];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, width as usize);
        });
        buffer
    }

    /// Counts pixels within `(x0..x1, y0..y1)` whose color differs from
    /// `background` by more than `threshold` per channel -- a cheap proxy
    /// for "something was actually drawn here" (text glyphs, in practice)
    /// without needing to read individual character shapes.
    fn count_non_background_pixels(
        buffer: &[Rgb8Pixel],
        width: u32,
        (x0, y0, x1, y1): (u32, u32, u32, u32),
        background: (u8, u8, u8),
        threshold: i32,
    ) -> usize {
        let differs = |a: u8, b: u8| (a as i32 - b as i32).abs() > threshold;
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = buffer[(y * width + x) as usize];
                differs(p.r, background.0) || differs(p.g, background.1) || differs(p.b, background.2)
            })
            .count()
    }

    /// Regression test for a bug where the output panel's `TextEdit` had
    /// `horizontal-stretch`/`vertical-stretch` set (which only has an
    /// effect inside an actual `Layout`) but was placed directly inside a
    /// plain `Rectangle`, so it silently rendered at ~0 size -- `run-log`
    /// was populated correctly the whole time (verified separately by
    /// `integration_tests`), but nothing was ever visible on screen. Reading
    /// properties back doesn't catch this class of bug; only checking
    /// actual rendered pixels does.
    #[test]
    fn recipe_output_is_visibly_rendered_not_just_populated() {
        let window = test_support::setup();

        let dir = std::env::temp_dir().join(format!("justgui-render-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("justfile"), "build:\n    echo hello world from build\n").unwrap();
        let dir = dir.to_string_lossy().into_owned();

        let ui = AppWindow::new().expect("failed to create Slint window");
        let state = Rc::new(RefCell::new(AppState {
            dir: dir.clone(),
            model: JustModel::default(),
            param_values: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
            active_session: -1,
            sessions_model: Rc::new(VecModel::default()),
            recipes_model: Rc::new(VecModel::default()),
            param_recipes_model: Rc::new(VecModel::default()),
            edit_dirty: false,
            edit_status: String::new(),
            theme: ThemeConfig::default(),
            editing_theme: false,
            layout: layout::LayoutConfig::default(),
            last_linted_buffer: String::new(),
            edit_lint_error: String::new(),
            env_file: envfile::EnvFile::default(),
        }));
        bind_models(&ui, &state.borrow());
        ui.set_directory(dir.clone().into());
        reload(&ui, &state);
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_run_recipe(move |idx| {
                if let Some(ui) = ui_handle.upgrade() {
                    run_recipe(&ui, &state, idx as usize);
                }
            });
        }

        let idx = state.borrow().model.recipes.iter().position(|r| r.name == "build").unwrap();
        ui.invoke_run_recipe(idx as i32);
        for _ in 0..30 {
            poll_sessions(&ui, &state);
            if ui.get_run_log().contains("hello world from build") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ui.get_run_log().contains("hello world from build"), "run_log never populated");

        let (width, height) = (1000, 700);
        let buffer = render(&window, width, height);

        // The output panel occupies roughly the bottom third of the default
        // 1000x700 window; sample a region comfortably inside it (below the
        // "Output" header row, above the stdin input row) against the
        // default `panel-background` (#2a2a2a). If the layout changes
        // meaningfully these coordinates may need adjusting.
        let non_background = count_non_background_pixels(
            &buffer,
            width,
            (20, 440, 980, 620),
            (0x2a, 0x2a, 0x2a),
            24,
        );
        assert!(
            non_background > 200,
            "expected visible text glyphs in the output panel, found {non_background} differing pixels"
        );

        let _ = std::fs::remove_dir_all(&state.borrow().dir);
    }

    /// Regression test for a bug where `poll_sessions` rebuilt the entire
    /// session-tab list model from scratch (`ui.set_sessions(ModelRc::new(...))`)
    /// on every 50ms tick, even when nothing had changed. Slint's `for`
    /// repeater treats a new `ModelRc` identity as a full reset and tears
    /// down/rebuilds every tab item, including whichever `TouchArea` a
    /// real mouse click's press landed on -- so any click whose release
    /// happened to land after the next tick (i.e. essentially every real
    /// click) got silently swallowed. `invoke_select_session` alone
    /// wouldn't have caught this: it calls the Rust callback directly and
    /// skips hit-testing entirely. This dispatches real
    /// `WindowEvent::Pointer*` events -- the same path an actual mouse
    /// click takes -- with a `poll_sessions` call spliced in between press
    /// and release, exactly like the real 50ms timer would during a click.
    #[test]
    fn session_tab_click_survives_a_poll_tick_between_press_and_release() {
        let window = test_support::setup();

        let dir = std::env::temp_dir().join(format!("justgui-click-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("justfile"), "stay-up:\n    sleep 5\n\nquick:\n    echo quick done\n").unwrap();
        let dir = dir.to_string_lossy().into_owned();

        let ui = AppWindow::new().expect("failed to create Slint window");
        let state = Rc::new(RefCell::new(AppState {
            dir: dir.clone(),
            model: JustModel::default(),
            param_values: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
            active_session: -1,
            sessions_model: Rc::new(VecModel::default()),
            recipes_model: Rc::new(VecModel::default()),
            param_recipes_model: Rc::new(VecModel::default()),
            edit_dirty: false,
            edit_status: String::new(),
            theme: ThemeConfig::default(),
            editing_theme: false,
            layout: layout::LayoutConfig::default(),
            last_linted_buffer: String::new(),
            edit_lint_error: String::new(),
            env_file: envfile::EnvFile::default(),
        }));
        bind_models(&ui, &state.borrow());
        ui.set_directory(dir.clone().into());
        reload(&ui, &state);
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_run_recipe(move |idx| {
                if let Some(ui) = ui_handle.upgrade() {
                    run_recipe(&ui, &state, idx as usize);
                }
            });
        }
        {
            let ui_handle = ui.as_weak();
            let state = state.clone();
            ui.on_select_session(move |id| {
                if let Some(ui) = ui_handle.upgrade() {
                    select_session(&ui, &state, id);
                }
            });
        }

        let stay_up_idx = state.borrow().model.recipes.iter().position(|r| r.name == "stay-up").unwrap();
        let quick_idx = state.borrow().model.recipes.iter().position(|r| r.name == "quick").unwrap();
        ui.invoke_run_recipe(stay_up_idx as i32);
        ui.invoke_run_recipe(quick_idx as i32);
        assert_eq!(ui.get_active_session(), state.borrow().sessions[1].id, "quick should start active");

        let (width, height) = (1000u32, 700u32);
        let _ = render(&window, width, height);

        // The session-tab strip sits directly below the Recipes/Edit-justfile
        // tab widget in the default 1000x700 window; (20, 365) lands inside
        // the first (leftmost) tab's select area, clear of its neighboring
        // close (x) button. If the layout changes meaningfully this may need
        // adjusting (see `recipe_output_is_visibly_rendered_not_just_populated`
        // above for the same tradeoff).
        use slint::platform::{PointerEventButton, WindowEvent};
        use slint::LogicalPosition;
        let click_x = 20.0f32;
        let mid_y = 365.0f32;
        ui.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(click_x, mid_y) });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: LogicalPosition::new(click_x, mid_y),
            button: PointerEventButton::Left,
        });
        // A real click's press-to-release span routinely crosses at least
        // one 50ms tick -- simulate that explicitly.
        poll_sessions(&ui, &state);
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(click_x, mid_y),
            button: PointerEventButton::Left,
        });

        let first_session_id = state.borrow().sessions[0].id;
        assert_eq!(
            ui.get_active_session(),
            first_session_id,
            "clicking the first tab should have selected it despite a poll tick between press and release"
        );

        let _ = std::fs::remove_dir_all(&state.borrow().dir);
    }
}

fn main() {
    let ui = AppWindow::new().expect("failed to create Slint window");

    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let state = Rc::new(RefCell::new(AppState {
        dir: dir.clone(),
        model: JustModel::default(),
        param_values: Vec::new(),
        sessions: Vec::new(),
        next_session_id: 0,
        active_session: -1,
        sessions_model: Rc::new(VecModel::default()),
        recipes_model: Rc::new(VecModel::default()),
        param_recipes_model: Rc::new(VecModel::default()),
        edit_dirty: false,
        edit_status: String::new(),
        theme: ThemeConfig::default(),
        editing_theme: false,
        layout: layout::LayoutConfig::default(),
        last_linted_buffer: String::new(),
        edit_lint_error: String::new(),
        env_file: envfile::EnvFile::default(),
    }));
    bind_models(&ui, &state.borrow());

    ui.set_directory(dir.into());
    reload(&ui, &state);

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_reload(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let dir = ui.get_directory().to_string();
                state.borrow_mut().dir = dir;
                reload(&ui, &state);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_run_recipe(move |idx| {
            if let Some(ui) = ui_handle.upgrade() {
                run_recipe(&ui, &state, idx as usize);
            }
        });
    }

    {
        let state = state.clone();
        ui.on_param_edited(move |ri, pi, text| {
            let mut st = state.borrow_mut();
            if let Some(row) = st.param_values.get_mut(ri as usize) {
                if let Some(slot) = row.get_mut(pi as usize) {
                    *slot = text.to_string();
                }
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_save_edit(move || {
            if let Some(ui) = ui_handle.upgrade() {
                save_edit(&ui, &state);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_edit_touched(move || {
            state.borrow_mut().edit_dirty = true;
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_edit_dirty(true);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_clear_log(move || {
            if let Some(ui) = ui_handle.upgrade() {
                clear_log(&ui, &state);
            }
        });
    }

    {
        let state = state.clone();
        ui.on_send_input(move |text| {
            send_input(&state, text.to_string());
        });
    }

    {
        let state = state.clone();
        ui.on_close_input(move || {
            close_input(&state);
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_select_session(move |id| {
            if let Some(ui) = ui_handle.upgrade() {
                select_session(&ui, &state, id);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_close_session(move |id| {
            if let Some(ui) = ui_handle.upgrade() {
                close_session(&ui, &state, id);
            }
        });
    }

    {
        let state = state.clone();
        ui.on_edit_externally(move || {
            edit_externally(&state);
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_show_private_toggled(move |_| {
            if let Some(ui) = ui_handle.upgrade() {
                let st = state.borrow();
                sync_ui(&ui, &st);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_toggle_shown(move |ri, shown| {
            if let Some(ui) = ui_handle.upgrade() {
                toggle_shown(&ui, &state, ri as usize, shown);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_recipe_dropped(move |dragged_idx, target_idx| {
            if let Some(ui) = ui_handle.upgrade() {
                recipe_dropped(&ui, &state, dragged_idx as usize, target_idx as usize);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_set_recipe_color(move |recipe_idx, color| {
            if let Some(ui) = ui_handle.upgrade() {
                set_recipe_color(&ui, &state, recipe_idx as usize, color);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_set_theme_color(move |field, color| {
            if let Some(ui) = ui_handle.upgrade() {
                set_theme_color(&ui, &state, field.as_str(), color);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_set_theme_field(move |field, value| {
            if let Some(ui) = ui_handle.upgrade() {
                set_theme_field(&ui, &state, field.as_str(), value.as_str());
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_save_theme(move || {
            if let Some(ui) = ui_handle.upgrade() {
                save_theme(&ui, &state);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_setting_value_edited(move |key, value| {
            if let Some(ui) = ui_handle.upgrade() {
                set_setting_value(&ui, &state, key.as_str(), value.as_str());
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_setting_bool_toggled(move |key, value| {
            if let Some(ui) = ui_handle.upgrade() {
                set_setting_value(&ui, &state, key.as_str(), if value { "true" } else { "false" });
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_add_setting(move |key| {
            if let Some(ui) = ui_handle.upgrade() {
                add_setting(&ui, &state, key.as_str());
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        ui.on_remove_setting(move |key| {
            if let Some(ui) = ui_handle.upgrade() {
                remove_setting(&ui, &state, key.as_str());
            }
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    poll_sessions(&ui, &state);
                }
            },
        );
    }

    let theme_timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        theme_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(750),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    refresh_theme(&ui, &state);
                }
            },
        );
    }

    let lint_timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        lint_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(600),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    lint_edit_buffer(&ui, &state);
                }
            },
        );
    }

    ui.run().expect("failed to run Slint event loop");
}
