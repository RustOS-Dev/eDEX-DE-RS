-- eDEX-DE user Hyprland configuration.
-- 1. The system defaults.
require("/usr/share/edex-de/hypr/hyprland")
-- 2. Values written by the eDEX-DE settings panel (missing until first saved).
local home = os.getenv("HOME") or ""
local xdg = os.getenv("XDG_CONFIG_HOME") or (home .. "/.config")
pcall(require, xdg .. "/edex-de/hypr/generated")
-- 3. Your own overrides. Create ~/.config/hypr/user.lua and put anything there.
pcall(require, xdg .. "/hypr/user")
