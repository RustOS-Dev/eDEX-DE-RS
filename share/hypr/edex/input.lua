hl.config({
    input = {
        kb_layout = "us",
        follow_mouse = 1,
        repeat_rate = 30,
        repeat_delay = 300,
        sensitivity = 0,
        touchpad = {
            natural_scroll = true,
            tap_to_click = true,
        },
    },
})

hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })
