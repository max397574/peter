use mlua::Lua;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use unicode_width::UnicodeWidthChar;

mod component;
mod components;
mod registry;

use component::Context;
use registry::Registry;

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

    let ctx = Context {
        cwd: env::current_dir().unwrap_or_default(),
        term_width,
        last_status,
        bind_mode,
        is_transient,
    };

    let lua = Lua::new();

    let get_cwd = lua.create_function(|_, ()| {
        let cwd = env::current_dir().unwrap_or_default();
        Ok(cwd.to_string_lossy().into_owned())
    })?;
    lua.globals().set("get_cwd", get_cwd)?;

    let displaywidth = lua.create_function(|_, text: String| Ok(display_width(&text)))?;
    lua.globals().set("displaywidth", displaywidth)?;

    lua.globals().set("last_status", ctx.last_status)?;
    lua.globals().set("bind_mode", ctx.bind_mode.clone())?;
    lua.globals().set("is_transient", ctx.is_transient)?;
    lua.globals().set("term_width", ctx.term_width)?;

    let mut registry = Registry::new();
    registry.register(Box::new(components::jj::component()));
    registry.register(Box::new(components::cwd::component()));
    registry.register(Box::new(components::lua::component()));
    registry.register(Box::new(components::rust::component()));

    let home_dir = env::var("HOME").expect("HOME environment variable must be set");
    let init_path = PathBuf::from(home_dir).join(".config/my_prompt/init.lua");

    let prompt_string: String = if init_path.exists() {
        let lua_code = fs::read_to_string(init_path).unwrap_or_default();
        registry::run_config(&lua, &mut registry, &ctx, &lua_code, display_width)?
    } else {
        String::from("\x1b[31m(config missing)\x1b[0m \u{276f} ")
    };

    let execution_time = start.elapsed();
    let prompt_string = prompt_string.replace("EXECUTION_TIME", &format!("{:?}", execution_time));

    print!("{prompt_string}");

    Ok(())
}
