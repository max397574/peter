use crate::component::{Context, ErasedComponent, LuaComponent, Segment};
use mlua::{Lua, Table, Value as LuaValue};
use std::collections::HashMap;

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
/// Lua may overwrite `.render` freely; that's plain Lua-table mutation
/// and doesn't call back into Rust until init.lua returns.
///
/// We deliberately do NOT expose `get_data` for built-in components: Lua
/// can change how a component is drawn, not how its data is fetched.
fn make_snapshot_table(lua: &Lua, name: &str, sentinel: &mlua::Function) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", name.to_string())?;
    table.set("render", sentinel.clone())?;
    Ok(table)
}

/// Reads the final ordered component list returned by init.lua and
/// applies any `render` overrides back onto the registry. Returns the
/// ordered list of component names to actually draw.
///
/// Each snapshot table's `render` starts out as a *sentinel* function
/// (identity-compared via `Function::equals`, which mlua implements as
/// Lua reference equality) rather than real render logic. If a table's
/// `render` is still that exact sentinel when init.lua returns, Lua never
/// touched it, so we skip set_lua_render and the component keeps using
/// its Rust default - avoiding rendering every component twice (once to
/// build a "default" closure, once for real) just to detect "unchanged".
fn apply_returned_components(
    registry: &mut Registry,
    returned: Table,
    sentinel: &mlua::Function,
) -> mlua::Result<Vec<String>> {
    let mut order = Vec::new();

    for pair in returned.sequence_values::<Table>() {
        let entry = pair?;
        let name: String = entry.get("name")?;
        let render_fn: mlua::Function = entry.get("render")?;
        // Function derives PartialEq via reference-identity comparison in
        // mlua (same underlying Lua object => equal), so this correctly
        // detects "Lua never reassigned .render" without a fallible call.
        let is_overridden = render_fn != *sentinel;

        match registry.get_mut(&name) {
            Some(component) => {
                if is_overridden {
                    component.set_lua_render(render_fn);
                }
            }
            None => {
                // Fully custom component defined from Lua (has its own
                // get_data too). If `get_data` is present, register it as
                // a new LuaComponent; otherwise this is an error - a
                // render override with no matching built-in and no
                // get_data has nothing to fetch data from.
                let get_data: mlua::Function = entry.get("get_data").map_err(|_| {
                    mlua::Error::RuntimeError(format!(
                        "component '{name}' has no matching built-in and no get_data function"
                    ))
                })?;
                let lua_component = LuaComponent {
                    name: name.clone(),
                    get_data,
                    render: render_fn,
                };
                registry.register(Box::new(lua_component));
            }
        }

        order.push(name);
    }

    Ok(order)
}

/// Runs init.lua (if present) with `get_component` wired up, and returns
/// the final ordered list of segment-groups to render, one Vec<Segment>
/// per component, in the order Lua specified.
pub fn run_config(
    lua: &Lua,
    registry: &mut Registry,
    ctx: &Context,
    lua_code: &str,
) -> mlua::Result<Vec<Vec<Segment>>> {
    // get_component(name) -> snapshot table
    //
    // We can't easily give this closure a live `&Registry` borrow (it'd
    // need to outlive the whole lua.load(...).eval() call, which also
    // wants &mut access later) so instead we snapshot ALL components up
    // front into a Lua-side table keyed by name, and get_component just
    // indexes into that. Mutations Lua makes to a table it already holds
    // (e.g. jj_component.render = ...) are plain Lua-table writes and
    // don't need to call back into Rust at all.
    // Sentinel: a no-op function used only for identity comparison later.
    // Never actually called - see apply_returned_components.
    let sentinel = lua.create_function(|_, ()| Ok(()))?;

    let snapshots = lua.create_table()?;
    for name in &registry.order {
        let table = make_snapshot_table(lua, name, &sentinel)?;
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
    let order = match returned {
        LuaValue::Table(t) => apply_returned_components(registry, t, &sentinel)?,
        _ => {
            return Err(mlua::Error::RuntimeError(
                "init.lua must return a list of components, e.g. `return {jj_component, cwd_component}`".into(),
            ))
        }
    };

    let mut rendered = Vec::with_capacity(order.len());
    for name in &order {
        let component = registry.get(name).ok_or_else(|| {
            mlua::Error::RuntimeError(format!("unknown component in result: '{name}'"))
        })?;
        if component.is_enabled(ctx) {
            rendered.push(component.render(ctx, lua)?);
        }
    }
    Ok(rendered)
}
