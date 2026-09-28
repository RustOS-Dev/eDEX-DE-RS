-- eDEX-DE system Hyprland configuration (Lua, Hyprland >= 0.55).
-- Installed as /usr/share/edex-de/hypr/edex/init.lua and loaded with `require("edex")` from the
-- user's ~/.config/hypr/hyprland.lua (see share/skel/hyprland.lua). Do not edit here: override in
-- ~/.config/hypr/user.lua or through the eDEX-DE settings panel, which writes
-- ~/.config/edex-de/hypr/generated.lua.
require("edex.env")
require("edex.monitors")
require("edex.look")
require("edex.input")
require("edex.rules")
require("edex.binds")
require("edex.autostart")
