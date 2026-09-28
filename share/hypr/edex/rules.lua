-- eDEX shell layers: never animate or blur them; the shell draws its own effects.
for _, ns in ipairs({ "canvas", "reserve-top", "reserve-bottom", "reserve-left", "reserve-right", "overlay", "toast" }) do
    hl.layer_rule({
        name = "edex-" .. ns,
        match = { namespace = "^edex-de:" .. ns .. "$" },
        no_anim = true,
    })
end
hl.layer_rule({ name = "edex-overlay-above-lock", match = { namespace = "^edex-de:toast$" }, above_lock = 1 })

-- Installer and dialogs float centred over the terminal slot.
hl.window_rule({ name = "calamares-float", match = { class = "^(calamares|edex-install)$" }, float = true, center = true, size = { "monitor_w*0.62", "monitor_h*0.75" } })
hl.window_rule({ name = "polkit-float", match = { class = "^(hyprpolkitagent|org.kde.polkit-kde-authentication-agent-1)$" }, float = true, center = true })
hl.window_rule({ name = "file-dialogs", match = { title = "^(Open File|Save File|Open Folder|Select Folder|Choose Files?)$" }, float = true, center = true, size = { "monitor_w*0.5", "monitor_h*0.6" } })
hl.window_rule({ name = "pavucontrol", match = { class = "^(org.pulseaudio.pavucontrol|pavucontrol)$" }, float = true, center = true })
hl.window_rule({ name = "xwayland-drag-fix", match = { class = "^$", title = "^$", xwayland = true, float = true, fullscreen = false, pin = false }, no_focus = true })
hl.window_rule({ name = "suppress-maximize", match = { class = ".*" }, suppress_event = "maximize" })
