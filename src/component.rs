use mlua::{FromLua, IntoLua, Lua, LuaSerdeExt, Value as LuaValue};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub struct Context {
    pub cwd: PathBuf,
    pub term_width: usize,
    pub last_status: i32,
    pub bind_mode: String,
    pub is_transient: bool,
}

fn dir_matches_pattern(dir: &Path, pattern: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    if let Some(ext) = pattern.strip_prefix("*.") {
        return entries.filter_map(Result::ok).any(|entry| {
            entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e == ext)
        });
    }

    if let Some(name) = pattern.strip_suffix('/') {
        return entries.filter_map(Result::ok).any(|entry| {
            entry.file_type().map(|t| t.is_dir()).unwrap_or(false) && entry.file_name() == name
        });
    }

    entries
        .filter_map(Result::ok)
        .any(|entry| entry.file_name() == pattern)
}

pub fn any_pattern_matches(dir: &Path, patterns: &[String]) -> bool {
    patterns.iter().any(|p| dir_matches_pattern(dir, p))
}

pub enum Dynamic<T> {
    Static(T),
    Dynamic(Box<dyn Fn(&Context) -> T>),
}

impl<T: Clone> Dynamic<T> {
    pub fn resolve(&self, ctx: &Context) -> T {
        match self {
            Dynamic::Static(v) => v.clone(),
            Dynamic::Dynamic(f) => f(ctx),
        }
    }
}

impl<T> From<T> for Dynamic<T> {
    fn from(v: T) -> Self {
        Dynamic::Static(v)
    }
}

impl<T> Dynamic<T> {
    pub fn new_dyn(f: impl Fn(&Context) -> T + 'static) -> Self {
        Dynamic::Dynamic(Box::new(f))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Color { r, g, b })
    }

    pub fn to_ansi_fg(self) -> String {
        format!("\x1b[38;2;{};{};{}m", self.r, self.g, self.b)
    }

    pub const RESET: &'static str = "\x1b[0m";
}

impl IntoLua for Color {
    fn into_lua(self, lua: &Lua) -> mlua::Result<LuaValue> {
        Ok(LuaValue::String(lua.create_string(format!(
            "#{:02X}{:02X}{:02X}",
            self.r, self.g, self.b
        ))?))
    }
}

impl FromLua for Color {
    fn from_lua(value: LuaValue, _lua: &Lua) -> mlua::Result<Self> {
        let s = match value {
            LuaValue::String(s) => s,
            other => {
                return Err(mlua::Error::FromLuaConversionError {
                    from: other.type_name(),
                    to: "Color".to_string(),
                    message: Some("expected a #RRGGBB hex string".into()),
                });
            }
        };
        let s: String = s.to_str()?.to_string();
        Color::from_hex(&s).ok_or_else(|| mlua::Error::FromLuaConversionError {
            from: "string",
            to: "Color".to_string(),
            message: Some(format!("invalid hex color: {s}")),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub text: String,
    pub color: Option<Color>,
}

impl Segment {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color: Some(color),
        }
    }
}

impl IntoLua for Segment {
    fn into_lua(self, lua: &Lua) -> mlua::Result<LuaValue> {
        let table = lua.create_table()?;
        table.set("text", self.text)?;
        match self.color {
            Some(c) => table.set("color", c.into_lua(lua)?)?,
            None => table.set("color", LuaValue::Nil)?,
        }
        Ok(LuaValue::Table(table))
    }
}

impl FromLua for Segment {
    fn from_lua(value: LuaValue, lua: &Lua) -> mlua::Result<Self> {
        let table = match value {
            LuaValue::Table(t) => t,
            other => {
                return Err(mlua::Error::FromLuaConversionError {
                    from: other.type_name(),
                    to: "Segment".to_string(),
                    message: Some("expected a { text = ..., color = ... } table".into()),
                });
            }
        };
        let text: String = table.get("text")?;
        let color: Option<Color> = match table.get::<LuaValue>("color")? {
            LuaValue::Nil => None,
            other => Some(Color::from_lua(other, lua)?),
        };
        Ok(Segment { text, color })
    }
}

pub fn render_segments(segments: &[Segment]) -> String {
    let mut out = String::new();
    for seg in segments {
        if let Some(color) = seg.color {
            out.push_str(&color.to_ansi_fg());
        }
        out.push_str(&seg.text);
        if seg.color.is_some() {
            out.push_str(Color::RESET);
        }
    }
    out
}

pub struct Component<D, C = ()> {
    pub name: String,
    pub enabled: Dynamic<bool>,
    pub get_data: Box<dyn Fn(&Context, &C) -> D>,
    pub render: Box<dyn Fn(&D) -> Vec<Segment>>,
    pub config: C,
    pub file_patterns: Vec<String>,
    render_override: Option<mlua::Function>,
}

impl<D, C: Default> Component<D, C> {
    pub fn new(
        name: impl Into<String>,
        get_data: impl Fn(&Context, &C) -> D + 'static,
        render: impl Fn(&D) -> Vec<Segment> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            enabled: Dynamic::Static(true),
            get_data: Box::new(get_data),
            render: Box::new(render),
            config: C::default(),
            file_patterns: Vec::new(),
            render_override: None,
        }
    }
}

pub trait ErasedComponent {
    fn name(&self) -> &str;
    fn is_enabled(&self, ctx: &Context) -> bool;
    fn render(&self, ctx: &Context, lua: &Lua) -> mlua::Result<Vec<Segment>>;
    fn set_lua_render(&mut self, f: mlua::Function);
    fn config_to_lua(&self, lua: &Lua) -> mlua::Result<LuaValue>;
    fn set_config_from_lua(&mut self, lua: &Lua, value: LuaValue) -> mlua::Result<()>;
}

impl<D, C> ErasedComponent for Component<D, C>
where
    D: Clone + Serialize + 'static,
    C: Serialize + serde::de::DeserializeOwned + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self, ctx: &Context) -> bool {
        if !self.enabled.resolve(ctx) {
            return false;
        }
        if self.file_patterns.is_empty() {
            return true;
        }
        any_pattern_matches(&ctx.cwd, &self.file_patterns)
    }

    fn render(&self, ctx: &Context, lua: &Lua) -> mlua::Result<Vec<Segment>> {
        let data = (self.get_data)(ctx, &self.config);
        match &self.render_override {
            Some(f) => {
                let lua_data = lua.to_value(&data)?;
                f.call::<Vec<Segment>>(lua_data)
            }
            None => Ok((self.render)(&data)),
        }
    }

    fn set_lua_render(&mut self, f: mlua::Function) {
        self.render_override = Some(f);
    }

    fn config_to_lua(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        lua.to_value(&self.config)
    }

    fn set_config_from_lua(&mut self, lua: &Lua, value: LuaValue) -> mlua::Result<()> {
        self.config = lua.from_value(value)?;
        Ok(())
    }
}

pub struct LuaComponent {
    pub name: String,
    pub get_data: Option<mlua::Function>,
    pub render: mlua::Function,
}

impl ErasedComponent for LuaComponent {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self, _ctx: &Context) -> bool {
        true
    }

    fn render(&self, _ctx: &Context, _lua: &Lua) -> mlua::Result<Vec<Segment>> {
        let data: LuaValue = match &self.get_data {
            Some(f) => f.call(())?,
            None => LuaValue::Nil,
        };
        self.render.call::<Vec<Segment>>(data)
    }

    fn set_lua_render(&mut self, f: mlua::Function) {
        self.render = f;
    }

    fn config_to_lua(&self, _lua: &Lua) -> mlua::Result<LuaValue> {
        Ok(LuaValue::Nil)
    }

    fn set_config_from_lua(&mut self, _lua: &Lua, _value: LuaValue) -> mlua::Result<()> {
        Ok(())
    }
}
