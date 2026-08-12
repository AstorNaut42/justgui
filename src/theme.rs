// Resolves a `[theme]` table from `justgui.toml` into concrete values the
// Slint UI can bind to. Lookup order: `<dir>/justgui.toml` (per-project),
// then a user-level config, then built-in defaults. Never fatal -- a
// missing or malformed file just falls back to the next candidate.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct ThemeConfig {
    pub mode: String, // "dark" or "light" -- controls built-in widget styling
    pub background: String,
    pub panel_background: String,
    pub border: String,
    pub accent: String,
    pub text: String,
    pub muted_text: String,
    pub error: String,
    pub warning: String,
    pub corner_radius: f32,
    pub font_family: String, // empty = system default
    pub font_size: f32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            mode: "dark".to_string(),
            background: "#1e1e1e".to_string(),
            panel_background: "#2a2a2a".to_string(),
            border: "#3c3c3c".to_string(),
            accent: "#7ec8ff".to_string(),
            text: "#e6e6e6".to_string(),
            muted_text: "#999999".to_string(),
            error: "#ff6666".to_string(),
            warning: "#ffaa33".to_string(),
            corner_radius: 6.0,
            font_family: String::new(),
            font_size: 14.0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfigFile {
    #[serde(default)]
    theme: ThemeConfig,
}

fn user_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(Path::new(&xdg).join("justgui").join("config.toml"));
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(Path::new(&appdata).join("justgui").join("config.toml"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(Path::new(&home).join(".config").join("justgui").join("config.toml"));
    }
    None
}

fn candidate_paths(dir: &str) -> Vec<PathBuf> {
    let mut v = vec![Path::new(dir).join("justgui.toml")];
    if let Some(p) = user_config_path() {
        v.push(p);
    }
    v
}

/// Re-reads and resolves the theme for `dir` from scratch. Cheap enough to
/// call on a timer for live hot-reload: a couple of `stat`s and a small
/// TOML parse.
pub fn resolve(dir: &str) -> ThemeConfig {
    for path in candidate_paths(dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match toml::from_str::<ConfigFile>(&text) {
            Ok(cfg) => return cfg.theme,
            Err(e) => {
                eprintln!("justgui: failed to parse {}: {e}", path.display());
                continue;
            }
        }
    }
    ThemeConfig::default()
}

/// Full overwrite of `<dir>/justgui.toml` -- only called from an explicit
/// Save action (the in-app theme editor), same accepted comment-loss
/// tradeoff `save_edit()` in main.rs already has for the justfile itself.
pub fn save(dir: &str, cfg: &ThemeConfig) -> std::io::Result<()> {
    let file = ConfigFile { theme: cfg.clone() };
    let text = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(Path::new(dir).join("justgui.toml"), text)
}

/// Parses `#RRGGBB` or `#RRGGBBAA` (leading `#` optional). Falls back to an
/// unmissable magenta on a malformed value, rather than panicking on a typo
/// in a user-edited config file.
pub fn parse_color(s: &str) -> slint::Color {
    let s = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
    if s.len() >= 6 && s.is_ascii() {
        let r = byte(0);
        let g = byte(2);
        let b = byte(4);
        let a = if s.len() >= 8 { byte(6) } else { 255 };
        slint::Color::from_argb_u8(a, r, g, b)
    } else {
        slint::Color::from_argb_u8(255, 255, 0, 255)
    }
}

/// Inverse of `parse_color`, for writing a color picked in the UI back into
/// a hex string (recipe accent colors, theme editor swatches).
pub fn color_to_hex(color: &slint::Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red(), color.green(), color.blue())
}
