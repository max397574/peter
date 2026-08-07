use mlua::Lua;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use unicode_width::UnicodeWidthChar;
mod component;
mod components;
use components::JJContext;

pub fn display_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            skip_escape_sequence(&mut chars);
            continue;
        }

        if c.is_control() {
            continue;
        }

        width += UnicodeWidthChar::width(c).unwrap_or(0);
    }

    width
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) {
    match chars.peek() {
        Some('[') => {
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            chars.next();
            loop {
                match chars.next() {
                    None | Some('\u{7}') => break,
                    Some('\u{1b}') => {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

fn main() -> mlua::Result<()> {
    let start = Instant::now();
    let args: Vec<String> = env::args().collect();
    let last_status: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let bind_mode = args.get(2).cloned().unwrap_or_else(|| "insert".to_string());
    let term_width: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(80);
    let is_transient = args.get(4).is_some_and(|s| s == "transient");

    let lua = Lua::new();
    lua.globals().set("last_status", last_status)?;
    lua.globals().set("bind_mode", bind_mode)?;
    lua.globals().set("is_transient", is_transient)?;
    lua.globals().set("term_width", term_width)?;

    let get_cwd = lua.create_function(|_, ()| {
        let cwd = env::current_dir().unwrap_or_default();
        Ok(cwd.to_string_lossy().into_owned())
    })?;
    lua.globals().set("get_cwd", get_cwd)?;

    let displaywidth = lua.create_function(|_, text: String| Ok(display_width(&text)))?;
    lua.globals().set("displaywidth", displaywidth)?;

    let get_jj_info = lua.create_function(|_, ()| {
        let root_check = Command::new("jj")
            .args(["--ignore-working-copy", "root"])
            .output();

        if root_check.is_err() || !root_check.unwrap().status.success() {
            return Ok(None);
        }

        let diff_output = Command::new("jj")
            .args(["diff", "--summary", "--ignore-working-copy"])
            .output();

        let mut files_added = 0;
        let mut files_modified = 0;
        let mut files_deleted = 0;
        let mut files_conflict = 0;

        if let Ok(output) = diff_output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.starts_with("A ") {
                    files_added += 1;
                } else if line.starts_with("M ") {
                    files_modified += 1;
                } else if line.starts_with("D ") {
                    files_deleted += 1;
                } else if line.starts_with("C ") {
                    files_conflict += 1;
                }
            }
        }

        let log_output = Command::new("jj")
            .args([
                "log",
                "--revisions",
                "@",
                "--no-graph",
                "--ignore-working-copy",
                "--limit",
                "1",
                "--template",
                r#"separate("\n", change_id.shortest(4), description)"#,
            ])
            .output();

        let mut change_id = String::from("????");
        let mut description = String::from("(no description set)");

        if let Ok(output) = log_output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            if let Some(id) = lines.first() {
                let trimmed = id.trim();
                if !trimmed.is_empty() {
                    change_id = trimmed.to_string();
                }
            }
            if lines.len() > 1 {
                let desc = lines[1..].join("\n").trim().to_string();
                if !desc.is_empty() {
                    description = desc;
                }
            }
        }

        Ok(Some(JJContext {
            change_id,
            files_added,
            files_modified,
            files_deleted,
            files_conflict,
            description,
        }))
    })?;

    lua.globals().set("get_jj_info", get_jj_info)?;

    let home_dir = env::var("HOME").expect("HOME environment variable must be set");
    let init_path = PathBuf::from(home_dir).join(".config/my_prompt/init.lua");

    let prompt_string: String = if init_path.exists() {
        let lua_code = fs::read_to_string(init_path).unwrap_or_default();
        lua.load(&lua_code).eval()?
    } else {
        String::from("\x1b[31m(config missing)\x1b[0m ❯ ")
    };

    let execution_time = start.elapsed();
    let prompt_string = prompt_string.replace("EXECUTION_TIME", &format!("{:?}", execution_time));

    print!("{prompt_string}");

    Ok(())
}
