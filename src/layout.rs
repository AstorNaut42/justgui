// Persists which recipes are shown on the Recipes tab, their display order,
// and accent color, per project. Lives in `justgui.layout.toml`, a sibling
// of `justgui.toml`, rather than inside `justgui.toml` itself: that file is
// hand-edited and hand-commented by the user for theming (see theme.rs), and
// the `toml` crate here has no comment-preserving round-trip, so writing
// this state into it on every drag/toggle would silently strip the user's
// comments and formatting. This file is entirely justgui-owned -- it's
// overwritten wholesale on every change, so there's nothing to lose.
use crate::just_client::JustRecipe;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A small fixed palette rather than a full color picker, to keep recipe
/// (and theme) coloring lightweight -- a row of swatches -- instead of
/// adding UI/dependency weight for something that's just an accent.
pub const PALETTE: &[&str] = &[
    "#7ec8ff", "#8fd6a0", "#ffaa33", "#ff6666", "#c792ea", "#f78c6c", "#82aaff", "#c3e88d",
];

fn default_color() -> String {
    PALETTE[0].to_string()
}

fn default_shown() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct RecipeViewEntry {
    pub name: String,
    #[serde(default = "default_shown")]
    pub shown: bool,
    pub position: i32, // flat display order, lower first
    #[serde(default = "default_color")]
    pub color: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct LayoutConfig {
    pub recipe: Vec<RecipeViewEntry>, // -> [[recipe]] tables
}

fn layout_path(dir: &str) -> PathBuf {
    Path::new(dir).join("justgui.layout.toml")
}

/// Loads `justgui.layout.toml` from `dir`. Missing file or parse failure
/// both just fall back to an empty layout -- never fatal, matching the
/// fallback style of `theme::resolve`.
pub fn load(dir: &str) -> LayoutConfig {
    let path = layout_path(dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return LayoutConfig::default();
    };
    match toml::from_str(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("justgui: failed to parse {}: {e}", path.display());
            LayoutConfig::default()
        }
    }
}

pub fn save(dir: &str, cfg: &LayoutConfig) -> std::io::Result<()> {
    let text = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(layout_path(dir), text)
}

/// Ensures every recipe has a layout entry (default shown, next linear
/// position, first palette color). Never prunes entries for recipes that
/// have temporarily disappeared from the justfile, so they keep their slot
/// and color if they reappear later.
pub fn sync_entries(cfg: &mut LayoutConfig, recipes: &[JustRecipe]) {
    let mut next_position = cfg.recipe.iter().map(|e| e.position).max().map_or(0, |p| p + 1);
    for recipe in recipes {
        if cfg.recipe.iter().any(|e| e.name == recipe.name) {
            continue;
        }
        cfg.recipe.push(RecipeViewEntry {
            name: recipe.name.clone(),
            shown: true,
            position: next_position,
            color: default_color(),
        });
        next_position += 1;
    }
}
