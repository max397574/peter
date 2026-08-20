use mlua::Lua;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Instant;

use cross_xdg::BaseDirs;

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

struct RenderArgs {
    last_status: i32,
    bind_mode: String,
    is_transient: bool,
    columns: usize,
}

fn parse_render_args(args: &[String]) -> RenderArgs {
    let mut last_status = 0;
    let mut columns = 0;
    let mut bind_mode = String::from("insert");
    let mut is_transient = false;

    for arg in args {
        if let Some(value) = arg.strip_prefix("--last-status=") {
            last_status = value.parse().unwrap_or(0);
        } else if let Some(value) = arg.strip_prefix("--columns=") {
            columns = value.parse().unwrap_or(0);
        } else if let Some(value) = arg.strip_prefix("--bind-mode=") {
            bind_mode = value.to_string();
        } else if arg == "--transient" {
            is_transient = true;
        }
    }

    RenderArgs {
        last_status,
        bind_mode,
        is_transient,
        columns,
    }
}

fn print_init_snippet(shell: &str) -> bool {
    match shell {
        "fish" => {
            print!(
                r#"set -g fish_transient_prompt 1
 
function fish_mode_prompt
    # hide vi mode indicator - peter_prompt draws its own
end
 
function fish_prompt
    set -l last_status $status
 
    if contains -- --final-rendering $argv
        peter_prompt --last-status=$last_status --bind-mode=$fish_bind_mode --columns=$COLUMNS --transient
    else
        peter_prompt --last-status=$last_status --bind-mode=$fish_bind_mode --columns=$COLUMNS
    end
end
"#
            );
            true
        }
        _ => todo!(),
    }
}

fn main() -> mlua::Result<()> {
    let start = Instant::now();
    let args: Vec<String> = env::args().collect();

    let mut registry = Registry::new();
    registry.register(Box::new(components::jj::component()));
    registry.register(Box::new(components::cwd::component()));
    registry.register(Box::new(components::lua::component()));
    registry.register(Box::new(components::rust::component()));

    match args.get(1).map(String::as_str) {
        Some("generate-annotations") => {
            let annotations = registry.generate_lua_annotations();
            match args.get(2) {
                Some(path) => {
                    fs::write(path, annotations).map_err(|e| {
                        mlua::Error::RuntimeError(format!("failed to write {path}: {e}"))
                    })?;
                    eprintln!("wrote annotations to {path}");
                }
                None => print!("{annotations}"),
            }
            return Ok(());
        }
        Some("init") => {
            let shell = args.get(2).map(String::as_str).unwrap_or("");
            if !print_init_snippet(shell) {
                std::process::exit(1);
            }
            return Ok(());
        }
        _ => {}
    }

    let render_args = parse_render_args(&args[1..]);

    let ctx = Context {
        cwd: env::current_dir().unwrap_or_default(),
        term_width: render_args.columns,
        last_status: render_args.last_status,
        bind_mode: render_args.bind_mode,
        is_transient: render_args.is_transient,
    };

    let lua = Lua::new();

    let globals = lua.globals();
    let package: mlua::Table = globals.get("package")?;
    let current_path: String = package.get("path")?;

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let custom_path = format!("{}/lua/?.lua", manifest_dir.replace("\\", "/"));

    let new_path = format!("{};{}", current_path, custom_path);
    package.set("path", new_path)?;

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

    let prompt_string = if let Ok(base_dirs) = BaseDirs::new() {
        let user_config_path = base_dirs.config_home().join("peter").join("init.lua");
        let fallback_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("default_config.lua");

        let config_code = if user_config_path.exists() {
            fs::read_to_string(&user_config_path)
        } else {
            fs::read_to_string(&fallback_path)
        };

        match config_code {
            Ok(lua_code) => {
                match registry::run_config(&lua, &mut registry, &ctx, &lua_code, display_width) {
                    Ok(prompt) => prompt,
                    Err(e) => format!("\x1b[31;1m[peter lua error: {}]\x1b[0m \u{276f} ", e),
                }
            }
            Err(e) => {
                format!("\x1b[31;1m[peter config error: {}]\x1b[0m \u{276f} ", e)
            }
        }
    } else {
        String::from("\x1b[31;1m[peter error: OS config directory missing]\x1b[0m \u{276f} ")
    };

    let execution_time = start.elapsed();
    let final_prompt = prompt_string.replace("EXECUTION_TIME", &format!("{:?}", execution_time));

    print!("{final_prompt}");
    Ok(())
}
