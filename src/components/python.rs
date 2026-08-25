use std::process::Command;

use crate::component::{Color, Component, Context, LuaAnnotated, Segment};
use serde::Serialize;

#[derive(Clone, Serialize, LuaAnnotated)]
pub struct PythonData {
    pub version: String,
}

fn get_data(_ctx: &Context, _config: &()) -> PythonData {
    let version = ["python", "python3", "python2"]
        .iter()
        .find_map(|&bin| {
            Command::new(bin)
                .arg("--version")
                .output()
                .ok()
                .and_then(|out| {
                    let output = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );

                    output.split_whitespace().nth(1).map(|s| s.to_owned())
                })
        })
        .unwrap_or_default();

    PythonData { version }
}

fn render(data: &PythonData) -> Vec<Segment> {
    let c_blue = Color::from_hex("#3776AB").unwrap();
    vec![Segment::new(format!("  {}", data.version), c_blue)]
}

pub fn component() -> Component<PythonData, ()> {
    let mut c = Component::new("python", get_data, render);
    c.file_patterns = vec![
        "*.py".to_string(),
        "pyproject.toml".to_string(),
        "requirements.txt".to_string(),
    ];
    c
}
