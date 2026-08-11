use crate::component::{Context, ErasedComponent, LuaComponent, render_segments};
use mlua::{Lua, Table, Value as LuaValue};
use std::collections::HashMap;

pub const ALIGN_MARKER: &str = "@align";

pub struct Registry {
    components: HashMap<String, Box<dyn ErasedComponent>>,
    order: Vec<String>,
}

fn header() -> String {
    format!(
        "-- LuaCATS type annotations for my_prompt, generated from the\n\
         -- Rust component definitions - do not edit by hand, regenerate\n\
         -- with `my_prompt --generate-annotations <path>`.\n\
         --\n\
         \n\
         ---@type integer Exit status of the last command\n\
         _G.last_status = 0\n\
         ---@type string Current shell bind mode (e.g. \"insert\", \"visual\", \"default\")\n\
         _G.bind_mode = \"\"\n\
         ---@type boolean True when the shell is redrawing the prompt for history\n\
         --- (e.g. Fish transient prompt), rather than a fresh render\n\
         _G.is_transient = false\n\
         ---@type integer Terminal width in columns, for \"{ALIGN_MARKER}\" padding\n\
         _G.term_width = 0\n\
         \n\
         ---@return string current working directory\n\
         function _G.get_cwd() end\n\
         \n\
         ---@param text string\n\
         ---@return integer display width of `text`, ANSI escapes excluded\n\
         function _G.displaywidth(text) end\n"
    )
}

impl Registry {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register(&mut self, component: Box<dyn ErasedComponent>) {
        let name = component.name().to_string();
        if !self.components.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.components.insert(name, component);
    }

    pub fn get(&self, name: &str) -> Option<&dyn ErasedComponent> {
        self.components.get(name).map(|c| c.as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn ErasedComponent>> {
        self.components.get_mut(name)
    }

    pub fn generate_lua_annotations(&self) -> String {
        let mut class_blocks = Vec::new();
        let mut overload_lines = Vec::new();

        for name in &self.order {
            let component = self
                .components
                .get(name)
                .expect("order stays in sync with components");
            let pascal_name = crate::component::to_pascal_case(name);
            let block = component.lua_annotations(&pascal_name);

            let (classes, overload) = block
                .rsplit_once('\n')
                .expect("lua_annotations always ends with a distinct ---@overload line");
            class_blocks.push(classes.to_string());
            overload_lines.push(overload.to_string());
        }

        let name_union = self
            .order
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join("|");

        format!(
            "{}\n{}\n\n---@alias MyPrompt.ComponentName {name_union}\n\n\
     {}\n---@param name MyPrompt.ComponentName\n---@return any\nfunction _G.get_component(name) end\n",
            header(),
            class_blocks.join("\n\n"),
            overload_lines.join("\n"),
        )
    }
}

fn make_snapshot_table(
    lua: &Lua,
    name: &str,
    component: &dyn ErasedComponent,
    sentinel: &mlua::Function,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", name.to_string())?;
    table.set("render", sentinel.clone())?;
    table.set("config", component.config_to_lua(lua)?)?;
    Ok(table)
}

enum ReturnedEntry {
    Component { name: String },
    Align,
}

fn apply_returned_components(
    lua: &Lua,
    registry: &mut Registry,
    returned: Table,
    sentinel: &mlua::Function,
) -> mlua::Result<Vec<ReturnedEntry>> {
    let mut order = Vec::new();

    for pair in returned.sequence_values::<LuaValue>() {
        let value = pair?;

        match value {
            LuaValue::String(s) => {
                let name = s.to_str()?.to_string();
                if name == ALIGN_MARKER {
                    order.push(ReturnedEntry::Align);
                } else {
                    if registry.get(&name).is_none() {
                        return Err(mlua::Error::RuntimeError(format!(
                            "'{name}' is not a known component (no matching built-in, and a bare string can't define get_data)"
                        )));
                    }
                    order.push(ReturnedEntry::Component { name });
                }
            }

            LuaValue::Table(entry) => {
                let name: String = entry.get("name")?;
                let render_fn: mlua::Function = entry.get("render")?;
                let is_overridden = render_fn != *sentinel;

                match registry.get_mut(&name) {
                    Some(component) => {
                        if is_overridden {
                            component.set_lua_render(render_fn);
                        }
                        match entry.get::<LuaValue>("config") {
                            Ok(LuaValue::Nil) | Err(_) => {}
                            Ok(config) => component.set_config_from_lua(lua, config)?,
                        }
                    }
                    None => {
                        let get_data: Option<mlua::Function> =
                            entry.get("get_data").unwrap_or(None);
                        let lua_component = LuaComponent {
                            name: name.clone(),
                            get_data,
                            render: render_fn,
                        };
                        registry.register(Box::new(lua_component));
                    }
                }

                order.push(ReturnedEntry::Component { name });
            }

            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "expected a component (table) or a component name / \"{ALIGN_MARKER}\" (string) in the returned list, got {}",
                    other.type_name()
                )));
            }
        }
    }

    Ok(order)
}

struct RenderedChunk {
    text: String,
    width: usize,
    align_right: bool,
}

pub fn run_config(
    lua: &Lua,
    registry: &mut Registry,
    ctx: &Context,
    lua_code: &str,
    display_width: impl Fn(&str) -> usize,
) -> mlua::Result<String> {
    let sentinel = lua.create_function(|_, ()| Ok(()))?;

    let snapshots = lua.create_table()?;
    for name in &registry.order {
        let component = registry
            .get(name)
            .expect("order is kept in sync with components");
        let table = make_snapshot_table(lua, name, component, &sentinel)?;
        snapshots.set(name.as_str(), table)?;
    }

    let snapshots_for_closure = snapshots.clone();
    let get_component = lua.create_function(move |_, name: String| match snapshots_for_closure
        .get::<LuaValue>(
        name.as_str(),
    )? {
        LuaValue::Nil => Err(mlua::Error::RuntimeError(format!(
            "no such component: '{name}'"
        ))),
        v => Ok(v),
    })?;
    lua.globals().set("get_component", get_component)?;

    let returned: LuaValue = lua.load(lua_code).eval()?;
    let entries = match returned {
        LuaValue::Table(t) => apply_returned_components(lua, registry, t, &sentinel)?,
        _ => {
            return Err(mlua::Error::RuntimeError(
                "init.lua must return a list of components, e.g. `return {jj_component, cwd_component}`".into(),
            ))
        }
    };

    let mut align_right = false;
    let mut lines: Vec<Vec<RenderedChunk>> = vec![Vec::new()];

    for entry in entries {
        match entry {
            ReturnedEntry::Align => align_right = true,
            ReturnedEntry::Component { name } => {
                let component = registry.get(&name).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("unknown component in result: '{name}'"))
                })?;
                if !component.is_enabled(ctx) {
                    continue;
                }
                let segments = component.render(ctx, lua)?;
                for part in split_on_newlines(&render_segments(&segments)) {
                    match part {
                        NewlinePart::Text(text) => {
                            let width = display_width(&text);
                            lines.last_mut().unwrap().push(RenderedChunk {
                                text,
                                width,
                                align_right,
                            });
                        }
                        NewlinePart::Newline => {
                            lines.push(Vec::new());
                            align_right = false; // alignment doesn't carry across lines
                        }
                    }
                }
            }
        }
    }

    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&render_line(line, ctx.term_width));
    }
    Ok(out)
}

enum NewlinePart {
    Text(String),
    Newline,
}

fn split_on_newlines(s: &str) -> Vec<NewlinePart> {
    let mut out = Vec::new();
    let mut chunks = s.split('\n').peekable();
    while let Some(chunk) = chunks.next() {
        if !chunk.is_empty() {
            out.push(NewlinePart::Text(chunk.to_string()));
        }
        if chunks.peek().is_some() {
            out.push(NewlinePart::Newline);
        }
    }
    out
}

fn render_line(chunks: &[RenderedChunk], term_width: usize) -> String {
    let (left, right): (Vec<_>, Vec<_>) = chunks.iter().partition(|c| !c.align_right);

    if right.is_empty() {
        return left
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("");
    }

    let left_text: String = left.iter().map(|c| c.text.as_str()).collect();
    let right_text: String = right.iter().map(|c| c.text.as_str()).collect();
    let left_width: usize = left.iter().map(|c| c.width).sum();
    let right_width: usize = right.iter().map(|c| c.width).sum();

    let padding = term_width.saturating_sub(left_width + right_width).max(1);
    format!("{left_text}{}{right_text}", " ".repeat(padding))
}
