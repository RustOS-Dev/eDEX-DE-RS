-- Session start: publish the environment to D-Bus/systemd, start the user services the
-- shell relies on, then the shell itself (as a user service so crashes restart it).
hl.on("hyprland.start", function()
    hl.exec_cmd("dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP HYPRLAND_INSTANCE_SIGNATURE QT_QPA_PLATFORM")
    hl.exec_cmd("systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP HYPRLAND_INSTANCE_SIGNATURE")
    hl.exec_cmd("systemctl --user restart xdg-desktop-portal-hyprland.service xdg-desktop-portal.service")
    hl.exec_cmd("systemctl --user start hyprpolkitagent.service hypridle.service")
    hl.exec_cmd("wl-paste --watch cliphist store")
    hl.exec_cmd("systemctl --user start edex-de.service")
end)

hl.on("hyprland.shutdown", function()
    hl.exec_cmd("systemctl --user stop edex-de.service hypridle.service")
end)
