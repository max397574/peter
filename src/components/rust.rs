use std::process::Command;

use crate::component::{Color, Component, Context, LuaAnnotated, Segment};
use serde::Serialize;

#[derive(Clone, Serialize, LuaAnnotated)]
pub struct RustData {
    pub version: String,
}

fn get_data(_ctx: &Context, _config: &()) -> RustData {
    let version = Command::new("rustc")
        .args(["--version"])
        .output()
        .map_or(String::new(), |out| {
            let output = String::from_utf8_lossy(&out.stdout);
            output.split_whitespace().nth(1).unwrap().to_owned()
        });
    RustData { version }
}

fn render(data: &RustData) -> Vec<Segment> {
    let c_blue = Color::from_hex("#CE412B").unwrap();
    vec![Segment::new(format!("  {}", data.version), c_blue)]
}

pub fn component() -> Component<RustData, ()> {
    let mut c = Component::new("rust", get_data, render);
    c.file_patterns = vec!["*.rs".to_string(), "Cargo.toml".to_string()];
    c
}
