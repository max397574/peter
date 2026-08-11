use crate::component::{Context, ErasedComponent, LuaComponent, Segment, render_segments};
use mlua::{Lua, Table, Value as LuaValue};
use std::collections::HashMap;

/// Marker string a user can place in the returned list to mark "align
/// what comes after this to the right, within the current line". Chosen
/// as a plain string (rather than a dedicated Lua object) so it composes
/// with bare-string component references - both are just strings in the
/// returned list, disambiguated by whether they match a component name.
pub const ALIGN_MARKER: &str = "@align";

/// Holds all built-in (Rust) components, keyed by name, plus any purely
/// Lua-defined ones registered during `init.lua` evaluation.
pub struct Registry {
    components: HashMap<String, Box<dyn ErasedComponent>>,
    /// Insertion order, used as the default ordering if init.lua doesn't
    /// return an explicit list.
    order: Vec<String>,
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
}

/// Converts a registry entry into the snapshot table handed to Lua by
/// `get_component`. `render` starts as a sentinel function (see
/// `apply_returned_components` for why) rather than real rendering logic.
/// `config` starts as the component's current (Rust-default, or
/// previously-Lua-set) config, serialized to a Lua table Lua can mutate
/// in place - e.g. `cwd_component.config.depth = 5`.
///
/// Lua may overwrite `.render` and mutate `.config` freely; both are
/// plain Lua-table operations and don't call back into Rust until
/// init.lua returns. We deliberately do NOT expose `get_data` for
/// built-in components: Lua can change how a component is drawn and
/// configured, not how its data is fetched.
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

/// A single entry from the list init.lua returned: either a real
/// component reference/definition, or the alignment marker.
enum ReturnedEntry {
    Component { name: String },
    Align,
}

/// Reads the final ordered component list returned by init.lua, applies
/// any `render`/`config` overrides back onto the registry, and returns
/// the ordered list of entries (component names and @align markers) to
/// render.
///
/// Each snapshot table's `render` starts out as a *sentinel* function
/// (compared via `Function`'s reference-identity `PartialEq`, which mlua
/// implements as Lua reference equality) rather than real render logic.
/// If a table's `render` is still that exact sentinel when init.lua
/// returns, Lua never touched it, so we skip set_lua_render and the
/// component keeps using its Rust default - avoiding rendering every
/// component twice (once to build a "default" closure, once for real)
/// just to detect "unchanged". `config` has no equivalent cheap-skip
/// trick (tables don't have an obvious "untouched" sentinel the way a
/// single function reference does) so it's always re-applied; this is
/// just a deserialize of a small table, cheap enough to not bother.
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
            // Bare string: either the align marker, or shorthand for
            // get_component(name) with no overrides at all - the user
            // just wants the built-in as-is, e.g. `return {"jj"}`.
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
                // Function derives PartialEq via reference-identity
                // comparison in mlua (same underlying Lua object =>
                // equal), so this correctly detects "Lua never
                // reassigned .render" without a fallible call.
                let is_overridden = render_fn != *sentinel;

                match registry.get_mut(&name) {
                    Some(component) => {
                        if is_overridden {
                            component.set_lua_render(render_fn);
                        }
                        // Only apply .config if Lua actually set/touched
                        // it - deserializing a whole struct from nil
                        // isn't guaranteed to fall back to Default the
                        // way a *missing field within a table* is (that
                        // fallback is what #[serde(default)] covers).
                        match entry.get::<LuaValue>("config") {
                            Ok(LuaValue::Nil) | Err(_) => {}
                            Ok(config) => component.set_config_from_lua(lua, config)?,
                        }
                    }
                    None => {
                        // Fully custom component defined from Lua, with
                        // no matching built-in. get_data is optional -
                        // if the render function doesn't need any data,
                        // there's no need to define one; render() will
                        // just receive nil.
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

/// One rendered, already-ANSI-joined chunk of a line, with a flag for
/// whether it should be pushed to the right edge of the terminal.
struct RenderedChunk {
    text: String,
    /// Raw (escape-code-stripped) display width of `text`, used for the
    /// right-alignment padding calculation.
    width: usize,
    align_right: bool,
}

/// Runs init.lua (if present) with `get_component` wired up, resolves
/// the returned entries into rendered, ANSI-joined text, and applies
/// @align padding within each line (a line = segments up to and
/// including one whose text contains '\n' - newlines are just embedded
/// in component text, same as the reference config did manually).
pub fn run_config(
    lua: &Lua,
    registry: &mut Registry,
    ctx: &Context,
    lua_code: &str,
    display_width: impl Fn(&str) -> usize,
) -> mlua::Result<String> {
    // get_component(name) -> snapshot table
    //
    // We can't easily give this closure a live `&Registry` borrow (it'd
    // need to outlive the whole lua.load(...).eval() call, which also
    // wants &mut access later) so instead we snapshot ALL components up
    // front into a Lua-side table keyed by name, and get_component just
    // indexes into that. Mutations Lua makes to a table it already holds
    // (e.g. jj_component.render = ..., cwd_component.config.depth = 5)
    // are plain Lua-table writes and don't need to call back into Rust
    // at all.
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

    // Render every entry into ANSI-joined chunks, tracking align-right
    // status and splitting into lines wherever a chunk's text contains
    // '\n'. A component whose rendered text spans a newline itself
    // (e.g. the old symbol_component's "\n " .. symbol) still works:
    // the split happens on the *rendered string*, not per-component.
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

/// Splits `s` on '\n', yielding the text between newlines interleaved
/// with explicit Newline markers (dropping empty leading/trailing text
/// pieces so `"\n"` yields just `[Newline]`, not `[Text(""), Newline]`).
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

/// Joins one line's chunks, inserting padding at the point @align
/// appeared so align_right chunks land flush against `term_width`. If
/// the combined width doesn't fit, falls back to a single-space gap
/// (matching the reference config's `format_right_aligned` behavior).
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
