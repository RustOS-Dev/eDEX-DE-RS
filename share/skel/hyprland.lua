-- eDEX-DE user Hyprland configuration (installed from /etc/skel).
-- Hyprland's `require` searches ~/.config/hypr and the Lua path, so the eDEX-DE modules are added
-- to the path first; EDEX_SHARE_DIR points at a development checkout when set.
local share = os.getenv("EDEX_SHARE_DIR")
local base = (share ~= nil and share ~= "") and (share .. "/hypr") or "/usr/share/edex-de/hypr"
package.path = base .. "/?.lua;" .. base .. "/?/init.lua;" .. package.path

-- 1. The system defaults (env, monitors, look, input, rules, binds, autostart).
require("edex")

-- 2. Values written by the eDEX-DE settings panel (missing until first saved).
local home = os.getenv("HOME") or ""
local xdg = os.getenv("XDG_CONFIG_HOME") or (home .. "/.config")
local function optional(path)
    local f = io.open(path, "r")
    if f == nil then
        return
    end
    f:close()
    local ok, err = pcall(dofile, path)
    if not ok then
        print("edex-de: error in " .. path .. ": " .. tostring(err))
    end
end
optional(xdg .. "/edex-de/hypr/generated.lua")

-- 3. Your own overrides: create ~/.config/hypr/user.lua and put anything there.
optional(xdg .. "/hypr/user.lua")
