# Architecture

## Processes

```
RustOS init ── svc ── service "edex" (tty1, root)
                        │
                        edex-comp --greeter ── Smithay: DRM/KMS + GBM/EGL (GLES), libinput, libseat → seatd
                        ├── edex-greeter          (xdg-toplevel; login over the control socket)
                        │      └── edex-comp runs `edex-auth check USER` → /storage/etc/shadow, /etc/shadow
                        │
                        └── after login, as the user (setuid, $XDG_RUNTIME_DIR=/run/user/UID):
                             Xwayland, dbus-daemon --session, pipewire, wireplumber, pipewire-pulse,
                             xdg-desktop-portal(-wlr), cliphist, edex-de run      (restarted when they crash)

edex-de ──(layer shell)── edex-comp            ──(D-Bus)── org.freedesktop.Notifications (served by edex-de)
   │  edex-comp.sock: requests + event stream        ├── worker thread: system backends (wpctl, svc, NM, …)
   │                                                 ├── PTY threads (alacritty_terminal event loops)
   └── edex-comp binds → $XDG_RUNTIME_DIR/edex-de/ipc.sock
apps ──(xdg-shell / Xwayland)── edex-comp: tiled into the app area edex-de reports
lock: edex-comp spawns `edex-greeter --lock` (ext-session-lock), which verifies through the control socket
```

edex-comp owns the seat, the outputs and the session; the shell is an ordinary (privileged by
socket, not by protocol) client. A shell crash leaves the windows and the session running, and the
supervisor restarts it.

## edex-comp

Based on Smithay's anvil (`edex-comp/LICENSE-anvil.txt`). The parts specific to eDEX:

| Module | Role |
|---|---|
| `wm.rs` | Pure window model: workspaces per output, dwindle/master tiling into the app area, maximize, fullscreen, floating, minimize, scratchpad, focus history. Unit tested. |
| `manage.rs` | Maps Smithay windows to the model, applies layouts (configure, map, raise), keeps focus, emits events. |
| `binds.rs`, `input_handler.rs` | Built-in and `[[wm.binds]]` key bindings, SUPER taps, repeat, mouse binds, lid switch. |
| `control.rs`, `ipc.rs` | The control socket: requests from `comp-proto`, an event stream made by diffing snapshots. |
| `lifecycle.rs` | Greeter → session, locking, idle (lock, DPMS), config reload. |
| `session.rs` | `/etc/passwd`, environment, setuid, and the supervisor that restarts session programs. |
| `udev.rs`, `winit.rs` | Backends: DRM/KMS with output rules, gamma (night light), DPMS and screenshots; nested for development. |

## Surfaces per output (edex-de)

| Surface | Layer | Purpose |
|---|---|---|
| `edex-de:canvas` | background, exclusive −1, keyboard on demand | The whole eDEX look, rendered with wgpu |
| `edex-de:strip` | top | The centre panel's tab strip (terminal and window tabs, window controls) above maximized windows |
| `edex-de:overlay` | overlay, exclusive keyboard | Launcher, settings, privacy, notifications, power (created on demand) |
| `edex-de:toast` | overlay, no keyboard | Toasts and OSD (created while there is something to show) |

After every layout change (panel resize, keyboard toggled, output added) the shell sends
`set-app-area` per output: the centre-panel rectangle under the tab strip for tiled windows and the
full-width rectangle for maximized ones. The terminal grid is sized from the primary output's
terminal rect using measured cell metrics.

## Crates

* `edex-comp` — the compositor (above) and its CLI (`msg`, `state`, `screenshot`).
* `comp-proto` — control-socket types and a blocking client with an event reader.
* `comp` — the shell's view of the compositor: `CompSocket` requests and `CompState` built from the
  event stream (tabs, workspaces, maximized windows).
* `edex-auth` — password checks and account edits on the RustOS shadow files.
* `platform` — sctk 0.21 client: outputs, layer surfaces, xdg window and session-lock surfaces
  (greeter), keyboard with repeat, pointer, cursor shapes, fractional scale, clipboard.
* `renderer` — `GpuContext` (instance/adapter/device, cosmic-text `FontSystem`) and `SurfaceRenderer`
  per surface: instanced SDF rectangles (panels, key caps, circles, hexagons, glow), scanlines,
  glyphon text with a content-keyed cache.
* `ui` — pure data: `Scene`, `HitMap`, `PanelLayout`, theme, widgets, panels and overlays. Snapshot
  tests pin the layout at several resolutions.
* `terminal` — tabs over `alacritty_terminal::Term`; key encoding, mouse reporting, selection, frame
  extraction into `ui::terminal_model::TerminalFrame`.
* `ipc` — the shell's newline-delimited JSON protocol, server and CLI parser.
* `settings` — config schema with defaults, atomic save, file watcher, GTK/Qt theme export.
* `launcher` — desktop entry scanning, fuzzy search with history, launching through edex-comp.
* `notifications` — zbus server for `org.freedesktop.Notifications` and the store/history.
* `sysmon` — sysinfo-based collector, battery, privacy probes.
* `system` — `CommandRunner` abstraction with real/fake runners; audio (wpctl), brightness (sysfs),
  network (NetworkManager D-Bus API), Bluetooth (`/dev/bluetooth`), power (UPower/login1), users
  (`edex-auth`), services (`svc`), display, input, privacy (Tor control port), about.
  `SystemBackend` runs requests on a worker thread.
* `edex-de` — the shell: event routing, rendering, input, overlays, forms, IPC handler.
* `edex-greeter` — login and lock screen UI, users and sessions.
