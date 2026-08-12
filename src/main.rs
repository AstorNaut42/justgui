// justgui -- a native GUI shell around `just`, built with Slint.
// Reads a justfile via `just --dump --dump-format json`, renders one
// button per recipe (with input fields for its parameters), streams the
// recipe's output live, and offers a plain-text editor for the justfile.
mod just_client;
mod layout;
mod process;
mod termout;
mod theme;

use just_client::{build_run_command, load_justfile, JustModel};
use process::Process;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use theme::ThemeConfig;

slint::include_modules!();

struct AppState {
    dir: String,
    model: JustModel,
    param_values: Vec<Vec<String>>, // per recipe, per param
    run_proc: Process,
    run_log: String,
    running_recipe: String, // non-empty while `run_proc` output belongs to a run
    edit_dirty: bool,
    edit_status: String,
    theme: ThemeConfig,
    editing_theme: bool, // true while a live (unsaved) theme edit is in effect
    layout: layout::LayoutConfig,
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
    RecipeData {
        recipe_index: idx as i32,
        name: r.name.clone().into(),
        doc: r.doc.clone().into(),
        is_private: r.is_private,
        is_shown: entry.is_none_or(|e| e.shown),
        color: theme::parse_color(entry.map_or(layout::PALETTE[0], |e| e.color.as_str())),
        params: ModelRc::new(VecModel::from(params)),
    }
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
    let recipes: Vec<RecipeData> = order
        .iter()
        .filter(|&&i| {
            let r = &st.model.recipes[i];
            let shown = st.layout.recipe.iter().find(|e| e.name == r.name).is_none_or(|e| e.shown);
            shown && (!r.is_private || show_private)
        })
        .map(|&i| recipe_data(st, i))
        .collect();

    ui.set_all_recipes(ModelRc::new(VecModel::from(all_recipes)));
    ui.set_recipes(ModelRc::new(VecModel::from(recipes)));
    ui.set_load_error(st.model.error.clone().into());
    ui.set_justfile_path(st.model.justfile_path.clone().into());
    ui.set_edit_dirty(st.edit_dirty);
    ui.set_edit_status(st.edit_status.clone().into());
    ui.set_run_log(st.run_log.clone().into());
    ui.set_running(st.run_proc.running() || !st.running_recipe.is_empty());
    ui.set_palette(ModelRc::new(VecModel::from(
        layout::PALETTE
            .iter()
            .map(|c| theme::parse_color(c))
            .collect::<Vec<_>>(),
    )));

    sync_theme_ui(ui, &st.theme);
}

fn reload(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    st.model = load_justfile(&st.dir);
    st.layout = layout::load(&st.dir);
    let AppState { layout, model, .. } = &mut *st;
    layout::sync_entries(layout, &model.recipes);
    let _ = layout::save(&st.dir, &st.layout);
    st.editing_theme = false;

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

fn run_recipe(ui: &AppWindow, state: &Rc<RefCell<AppState>>, idx: usize) {
    let mut st = state.borrow_mut();
    if st.run_proc.running() {
        return;
    }
    let Some(recipe) = st.model.recipes.get(idx).cloned() else {
        return;
    };
    let Some(param_values) = st.param_values.get(idx).cloned() else {
        return;
    };

    let cmd = build_run_command(&recipe, &param_values);
    st.run_log = format!("$ {cmd}\n");
    st.running_recipe = recipe.name.clone();
    let dir = st.dir.clone();
    st.run_proc.start(&cmd, &dir);

    ui.set_run_log(st.run_log.clone().into());
    ui.set_running(true);
}

fn poll_log(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    if st.running_recipe.is_empty() && !st.run_proc.running() {
        return;
    }

    let (chunk, finished) = st.run_proc.poll();
    if !chunk.is_empty() {
        termout::append_chunk(&mut st.run_log, &chunk);
    }
    if finished && !st.running_recipe.is_empty() {
        let code = st.run_proc.exit_code();
        st.run_log.push_str(&format!("\n[exit code {code}]\n"));
        st.running_recipe.clear();
    }

    let running = st.run_proc.running() || !st.running_recipe.is_empty();
    ui.set_run_log(st.run_log.clone().into());
    ui.set_running(running);
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

/// Sends a line to the running recipe's pty stdin. The pty's own line
/// discipline echoes it back through the normal output stream (like a real
/// terminal would), so it shows up in the log via the next `poll_log` --
/// no need to echo it here ourselves.
fn send_input(state: &Rc<RefCell<AppState>>, text: String) {
    let st = state.borrow();
    if !st.run_proc.running() {
        return;
    }
    st.run_proc.send_input(&text);
}

fn close_input(state: &Rc<RefCell<AppState>>) {
    state.borrow().run_proc.close_input();
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
        run_proc: Process::new(),
        run_log: String::new(),
        running_recipe: String::new(),
        edit_dirty: false,
        edit_status: String::new(),
        theme: ThemeConfig::default(),
        editing_theme: false,
        layout: layout::LayoutConfig::default(),
    }));

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
            state.borrow_mut().run_log.clear();
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_run_log("".into());
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

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let state = state.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    poll_log(&ui, &state);
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

    ui.run().expect("failed to run Slint event loop");
}
