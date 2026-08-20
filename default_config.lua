local theme = require("peter.themes.onedark")

if is_transient then
    return {
        {
            name = "transient_symbol",
            render = function()
                return { { text = "  ", color = last_status ~= 0 and theme.red or theme.green } }
            end,
        },
    }
end

local cwd_component = get_component("cwd")
cwd_component.config.substitutions = { [os.getenv("HOME")] = "~" }

local symbol_component = {
    name = "prompt_symbol",
    render = function()
        return { { text = "\n  ", color = last_status ~= 0 and theme.red or theme.green } }
    end,
}

local clock_component = {
    name = "clock",
    render = function()
        return {
            { text = os.date("%H:%M:%S"), color = theme.blue },
        }
    end,
}

local function space(n)
    return {
        name = "space" .. n,
        render = function()
            return { { text = string.rep(" ", n) } }
        end,
    }
end

return { cwd_component, space(1), "jj", "lua", "rust", "@align", clock_component, symbol_component }
