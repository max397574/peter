use crate::component::{Color, Component, Context, LuaAnnotated, Segment};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize, LuaAnnotated)]
#[serde(default)]
pub struct CwdConfig {
    /// Depth of directories to show (upwards)
    pub depth: usize,
    /// Substitute parts of the path with another string
    pub substitutions: HashMap<String, String>,
}

impl Default for CwdConfig {
    fn default() -> Self {
        Self {
            depth: 3,
            substitutions: HashMap::new(),
        }
    }
}

#[derive(Clone, Serialize, LuaAnnotated)]
pub struct CwdData {
    /// The path shortened as per config.depth
    pub short_path: String,
}

fn get_short_path(path: &str, depth: usize) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let start = parts.len().saturating_sub(depth);
    parts[start..].join("/")
}

fn apply_substitutions(mut text: String, subs: &HashMap<String, String>) -> String {
    for (lhs, rhs) in subs {
        text = text.replace(lhs.as_str(), rhs.as_str());
    }
    text
}

fn get_cwd_data(ctx: &Context, config: &CwdConfig) -> CwdData {
    let raw = ctx.cwd.to_string_lossy().into_owned();
    let substituted = apply_substitutions(raw, &config.substitutions);
    CwdData {
        short_path: get_short_path(&substituted, config.depth),
    }
}

fn render_cwd(data: &CwdData) -> Vec<Segment> {
    let c_orange = Color::from_hex("#FFA500").unwrap();
    vec![Segment::new(format!(" {}", data.short_path), c_orange)]
}

pub fn component() -> Component<CwdData, CwdConfig> {
    Component::new("cwd", get_cwd_data, render_cwd)
}
