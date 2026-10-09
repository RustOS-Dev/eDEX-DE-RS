# Architecture

## Process model

```
Hyprland ──(layer shell)── edex-de ──(D-Bus)── org.freedesktop.Notifications server
   │  .socket.sock / .socket2.sock          ├── worker thread: system backends (wpctl, nmcli, …)
   │                                        ├── PTY threads (alacritty_terminal event loops)
   └── binds → `edex-de ipc …` ─────────────┴── $XDG_RUNTIME_DIR/edex-de/ipc.sock
```

`edex-de` is one process with a calloop event loop (`platform` crate). Threads (PTYs, D-Bus, system
backends) send `AppEvent`s through a calloop channel; the Hyprland event socket and the IPC listener are
level-triggered fd sources; timers drive the clock, sysmon, privacy probes, animation and toast expiry.

## Surfaces per output

| Surface | Layer | Purpose |
|---|---|---|
| `edex-de:canvas` | background, exclusive −1, keyboard on-demand | The whole eDEX look, rendered with wgpu |
| `edex-de:reserve-{top,bottom,left,right}` | top, exclusive = bar/keyboard/panel size | 1×1 transparent buffer scaled with `wp_viewport`, empty input region; they make Hyprland tile windows into the terminal slot |
| `edex-de:overlay` | overlay, exclusive keyboard | Launcher, settings, privacy, notifications, power (created on demand) |
| `edex-de:toast` | overlay, no keyboard | Toasts and OSD (created while there is something to show) |

Dragging a resize handle changes the reserver sizes and Hyprland relayouts live. The terminal grid is
sized from the primary output's terminal rect using measured cell metrics.

## Crates

* `platform` — sctk 0.21 client: outputs, layer surfaces, xdg window (greeter), keyboard with repeat,
  pointer, cursor shapes, fractional scale, clipboard via the data device, and the compositor's
  window list through `zwlr_foreign_toplevel_management_v1`.
* `renderer` — `GpuContext` (instance/adapter/device, cosmic-text `FontSystem`) and `SurfaceRenderer`
  per surface: instanced SDF rectangles (panels, key caps, circles, hexagons, glow), scanlines, glyphon text
  with a content-keyed cache.
* `ui` — pure data: `Scene`, `HitMap`, `PanelLayout`, theme, widgets, panels and overlays. Snapshot tests
  pin the layout at several resolutions.
* `terminal` — tabs over `alacritty_terminal::Term`; key encoding, mouse reporting, selection, frame
  extraction into `ui::terminal_model::TerminalFrame`.
* `hypr` — IPC socket client (`j/` JSON requests, dispatch, eval, reload) and event stream parser.
* `wm` — the window-manager backend trait (`WindowManager`) and the generic window/workspace model
  the tab strip uses; backends: Hyprland (over `hypr`), labwc/wlroots (foreign-toplevel windows from
  `platform::Toplevels`, rc.xml export, reconfigure and exit through `LABWC_PID`) and none.
* `ipc` — newline-delimited JSON protocol, server and CLI parser.
* `settings` — config schema with defaults, atomic save, file watcher, Lua/hypridle export.
* `launcher` — desktop entry scanning, fuzzy search with history, detached launching via Hyprland.
* `notifications` — zbus server for `org.freedesktop.Notifications` and the store/history.
* `sysmon` — sysinfo-based collector, battery, privacy probes.
* `system` — `CommandRunner` abstraction with real/fake runners; `Os` detection and `Capabilities`
  (rows without a backend are hidden); the `rustos` backend set (ALSA, `wifi`/`ip`, `bt`, /sys, /etc/rc); audio, brightness, network, bluetooth,
  power (upower/logind over zbus), users, services, display, input, privacy (tor control port, tailscale),
  fprintd (zbus), about. `SystemBackend` runs requests on a worker thread.
* `edex-de` — the application: event routing, rendering, input, overlays, forms, IPC handler.
* `edex-greeter` — greetd client, users/sessions, fullscreen login UI.
