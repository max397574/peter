---@meta
-- Type annotations for Peter, generated from Rust components
-- Don't edit manually, regenerate with `my_prompt --generate-annotations <path>`.

---@type integer Exit status of the last command
_G.last_status = 0
---@type string Current shell bind mode (e.g. "insert", "visual", "default")
_G.bind_mode = ""
---@type boolean
_G.is_transient = false
---@type integer Terminal width in columns
_G.term_width = 0

---@return string Current working directory
function _G.get_cwd() end

---@param text string
---@return integer Display width of `text`
function _G.displaywidth(text) end

---@alias Peter.Segment { [1]: string, [2]: string }
---@class Peter.Jj.Config

---@class Peter.Jj.Data
---@field change_id string
---@field files_added integer
---@field files_modified integer
---@field files_deleted integer
---@field files_conflict integer
---@field description string First line of the current change's description

---@class Peter.Jj.Component
---@field config Peter.Jj.Config
---@field render fun(data: Peter.Jj.Data): Peter.Segment[]


---@class Peter.Cwd.Config
---@field depth integer Depth of directories to show (upwards)
---@field substitutions table<string, string> Substitute parts of the path with another string

---@class Peter.Cwd.Data
---@field short_path string The path shortened as per config.depth

---@class Peter.Cwd.Component
---@field config Peter.Cwd.Config
---@field render fun(data: Peter.Cwd.Data): Peter.Segment[]


---@alias LuaType "Lua"|"LuaJIT"

---@class Peter.Lua.Config
---@field lua_type LuaType

---@class Peter.Lua.Data
---@field version string

---@class Peter.Lua.Component
---@field config Peter.Lua.Config
---@field render fun(data: Peter.Lua.Data): Peter.Segment[]


---@class Peter.Rust.Config

---@class Peter.Rust.Data
---@field version string

---@class Peter.Rust.Component
---@field config Peter.Rust.Config
---@field render fun(data: Peter.Rust.Data): Peter.Segment[]


---@alias Peter.ComponentName "jj"|"cwd"|"lua"|"rust"

---@overload fun(name: "jj"): Peter.Jj.Component
---@overload fun(name: "cwd"): Peter.Cwd.Component
---@overload fun(name: "lua"): Peter.Lua.Component
---@overload fun(name: "rust"): Peter.Rust.Component
---@param name Peter.ComponentName
---@return any
function _G.get_component(name) end
