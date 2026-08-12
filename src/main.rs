// justgui -- a native GUI shell around `just`, built with Slint.
// Reads a justfile via `just --dump --dump-format json`, renders one
// button per recipe (with input fields for its parameters), streams the
// recipe's output live, and offers a plain-text editor for the justfile.
mod just_client;
mod process;

use just_client::{build_run_command, load_justfile, JustModel};
use process::Process;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

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

fn sync_ui(ui: &AppWindow, st: &AppState) {
    let recipes: Vec<RecipeData> = st
        .model
        .recipes
        .iter()
        .map(|r| {
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
                name: r.name.clone().into(),
                doc: r.doc.clone().into(),
                is_private: r.is_private,
                params: ModelRc::new(VecModel::from(params)),
            }
        })
        .collect();

    ui.set_recipes(ModelRc::new(VecModel::from(recipes)));
    ui.set_load_error(st.model.error.clone().into());
    ui.set_justfile_path(st.model.justfile_path.clone().into());
    ui.set_edit_dirty(st.edit_dirty);
    ui.set_edit_status(st.edit_status.clone().into());
    ui.set_run_log(st.run_log.clone().into());
    ui.set_running(st.run_proc.running() || !st.running_recipe.is_empty());
}

fn reload(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    st.model = load_justfile(&st.dir);

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
        st.run_log.push_str(&chunk);
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

    ui.run().expect("failed to run Slint event loop");
}
