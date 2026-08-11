use mlua::{FromLua, IntoLua, Lua, LuaSerdeExt, Value as LuaValue};
use serde::Serialize;
use std::path::PathBuf;

pub struct Context {
    pub cwd: PathBuf,
    pub term_width: usize,
    pub last_status: i32,
    pub bind_mode: String,
    pub is_transient: bool,
}

// ---------------------------------------------------------------------
// Dynamic<T>: a value that's either fixed at construction, or computed
// from Context each time it's needed (e.g. `enabled`).
// ---------------------------------------------------------------------

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
    pub fn dynamic(f: impl Fn(&Context) -> T + 'static) -> Self {
        Dynamic::Dynamic(Box::new(f))
    }
}

// ---------------------------------------------------------------------
// Color: always a #RRGGBB hex string underneath. Reset is injected
// automatically by the renderer, never chosen by component authors.
// ---------------------------------------------------------------------

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
        // Convert to an owned String rather than relying on the exact
        // borrow-vs-owned shape mlua's String::to_str() returns.
        let s: String = s.to_str()?.to_string();
        Color::from_hex(&s).ok_or_else(|| mlua::Error::FromLuaConversionError {
            from: "string",
            to: "Color".to_string(),
            message: Some(format!("invalid hex color: {s}")),
        })
    }
}

// ---------------------------------------------------------------------
// Segment: one piece of output text plus its color. The renderer joins
// segments, converts colors to ANSI, and injects a reset after each one
// automatically - component/render authors never write escape codes.
// ---------------------------------------------------------------------

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

    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
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

/// Joins segments into final printable output: ANSI color, text, ANSI
/// reset, for every segment. This is the "concat + escape sequence" step
/// done centrally in Rust so neither Rust component authors nor Lua
/// render overrides ever have to write escape codes themselves.
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

// ---------------------------------------------------------------------
// Component<D>: the concrete, typed component definition Rust code
// builds. `render_override` is filled in by Lua via ErasedComponent's
// set_lua_render; when present it wins over the Rust `render` closure,
// but `get_data` is always the Rust closure - Lua never replaces how a
// *built-in* component's data is fetched, only how it's displayed.
// ---------------------------------------------------------------------

pub struct Component<D> {
    pub name: String,
    pub enabled: Dynamic<bool>,
    pub get_data: Box<dyn Fn(&Context) -> D>,
    pub render: Box<dyn Fn(&D) -> Vec<Segment>>,
    pub file_patterns: Vec<String>,
    // mlua 0.10 dropped the 'lua lifetime (Function now holds a weak ref
    // to Lua internally), so this can just be stored directly - no
    // RegistryKey indirection needed.
    render_override: Option<mlua::Function>,
}

impl<D> Component<D> {
    pub fn new(
        name: impl Into<String>,
        get_data: impl Fn(&Context) -> D + 'static,
        render: impl Fn(&D) -> Vec<Segment> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            enabled: Dynamic::Static(true),
            get_data: Box::new(get_data),
            render: Box::new(render),
            file_patterns: Vec::new(),
            render_override: None,
        }
    }
}

/// Type-erased handle so the registry can hold `Component<JJData>`,
/// `Component<CwdData>`, etc. uniformly, and so Lua can install a render
/// override without the registry itself needing to be generic.
pub trait ErasedComponent {
    fn name(&self) -> &str;
    fn is_enabled(&self, ctx: &Context) -> bool;
    /// Full pipeline: fetch data, then render (Lua override if present,
    /// else the Rust default). This is the only entry point the final
    /// prompt assembly needs.
    fn render(&self, ctx: &Context, lua: &Lua) -> mlua::Result<Vec<Segment>>;
    /// Install/replace a Lua render function for this component. Only
    /// affects rendering; `get_data` (for Rust-defined components) is
    /// untouched.
    fn set_lua_render(&mut self, f: mlua::Function);
}

impl<D> ErasedComponent for Component<D>
where
    D: Clone + Serialize + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self, ctx: &Context) -> bool {
        self.enabled.resolve(ctx)
    }

    fn render(&self, ctx: &Context, lua: &Lua) -> mlua::Result<Vec<Segment>> {
        let data = (self.get_data)(ctx);
        match &self.render_override {
            Some(f) => {
                // serde -> LuaValue, instead of a hand-written IntoLua impl
                // per component data type. Every Component<D>'s D need only
                // derive(Serialize) to be usable from a Lua render override.
                let lua_data = lua.to_value(&data)?;
                f.call::<Vec<Segment>>(lua_data)
            }
            None => Ok((self.render)(&data)),
        }
    }

    fn set_lua_render(&mut self, f: mlua::Function) {
        self.render_override = Some(f);
    }
}

// ---------------------------------------------------------------------
// LuaComponent: a component defined entirely from Lua (both get_data
// and render are Lua functions). Used for components with no Rust
// built-in - e.g. a fully custom user segment.
// ---------------------------------------------------------------------

pub struct LuaComponent {
    pub name: String,
    pub get_data: mlua::Function,
    pub render: mlua::Function,
}

impl ErasedComponent for LuaComponent {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self, _ctx: &Context) -> bool {
        // Lua-defined components decide "nothing to show" via render
        // returning an empty segment list, rather than a separate
        // enabled check - keeps the Lua-side contract to two functions.
        true
    }

    fn render(&self, _ctx: &Context, _lua: &Lua) -> mlua::Result<Vec<Segment>> {
        let data: LuaValue = self.get_data.call(())?;
        self.render.call::<Vec<Segment>>(data)
    }

    fn set_lua_render(&mut self, f: mlua::Function) {
        self.render = f;
    }
}
