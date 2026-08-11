use std::process::Command;

use crate::component::{Color, Component, Context, LuaAnnotated, Segment};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub enum LuaType {
    Lua,
    LuaJIT,
}

impl LuaType {
    pub fn binary_name(&self) -> String {
        match self {
            LuaType::LuaJIT => "luajit".to_string(),
            LuaType::Lua => "lua".to_string(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, LuaAnnotated)]
#[serde(default)]
pub struct LuaConfig {
    pub lua_type: LuaType,
}

impl Default for LuaConfig {
    fn default() -> Self {
        Self {
            lua_type: LuaType::Lua,
        }
    }
}

#[derive(Clone, Serialize, LuaAnnotated)]
pub struct LuaData {
    pub version: String,
}

fn get_data(_ctx: &Context, config: &LuaConfig) -> LuaData {
    let version = Command::new(config.lua_type.binary_name())
        .args(["-v"])
        .output()
        .map_or(String::new(), |out| {
            let output = String::from_utf8_lossy(&out.stdout);
            output.split_whitespace().nth(1).unwrap().to_owned()
        });
    LuaData { version }
}

fn render(data: &LuaData) -> Vec<Segment> {
    let c_blue = Color::from_hex("#4E68B2").unwrap();
    vec![Segment::new(format!("  {}", data.version), c_blue)]
}

pub fn component() -> Component<LuaData, LuaConfig> {
    let mut c = Component::new("lua", get_data, render);
    c.file_patterns = vec!["*.lua".to_string(), "lua/".to_string()];
    c
}
