-- eDEX-DE system Hyprland configuration (Lua, Hyprland >= 0.55).
-- Installed to /usr/share/edex-de/hypr/hyprland.lua and required by the user's
-- ~/.config/hypr/hyprland.lua. Do not edit here: override in ~/.config/hypr/user.lua or
-- through the eDEX-DE settings panel (which writes ~/.config/edex-de/hypr/generated.lua).

local base = "/usr/share/edex-de/hypr/"
local share = os.getenv("EDEX_SHARE_DIR")
if share ~= nil and share ~= "" then
    base = share .. "/hypr/"
end

require(base .. "env")
require(base .. "monitors")
require(base .. "look")
require(base .. "input")
require(base .. "rules")
require(base .. "binds")
require(base .. "autostart")
