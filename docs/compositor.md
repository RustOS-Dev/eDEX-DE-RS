# edex-comp

The eDEX compositor: a Smithay-based Wayland compositor that replaces Hyprland. It manages
windows for the shell (tiling into the centre panel, the tab strip's window controls, workspaces),
runs the login screen and the user's session, and locks it.

## Running

```text
edex-comp [run] [--greeter] [--nested] [--run COMMAND]…
edex-comp msg '{"cmd":"state"}'
edex-comp state
edex-comp screenshot [--output NAME] [--region "X,Y WxH"] FILE.png
```

* `run` on a VT takes the seat through libseat (seatd), opens every DRM device, and starts the
  session of the user running it: Xwayland and the programs below.
* `--greeter` (the RustOS `edex` service, as root on tty1) shows `edex-greeter` first and starts the
  session of whoever logs in.
* `--nested` runs in a window of another Wayland or X11 session, for development.
* `--run COMMAND` starts these programs instead of the eDEX session.

The session programs, each restarted when it crashes (at most five times a minute):

| Program | Started when installed |
|---|---|
| `dbus-daemon --session` at `$XDG_RUNTIME_DIR/bus` | yes |
| `pipewire`, `wireplumber`, `pipewire-pulse` | yes |
| `/usr/libexec/xdg-desktop-portal`, `xdg-desktop-portal-wlr` | yes |
| `wl-paste --watch cliphist store` | yes |
| `edex-de run` | always |

Every program gets `WAYLAND_DISPLAY`, `DISPLAY` (Xwayland), `XDG_RUNTIME_DIR`,
`XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=eDEX-DE`, `DBUS_SESSION_BUS_ADDRESS` and
`EDEX_COMP_SOCKET`.

## Window management

* **App area.** The shell reports, per output, the rectangle under its tab strip between the side
  panels (`tiled`) and the same strip at full width (`maximized`). Tiled windows split the tiled
  area (dwindle or master layout, `wm.gaps_in`/`gaps_out`); maximized windows fill the maximized
  area while the shell hides its side panels; fullscreen windows cover the output above the shell.
* **Workspaces** 1..`wm.workspaces` are shown one per output; switching to one shown elsewhere
  focuses that output. Minimized windows live on a hidden workspace and come back where you are
  looking. `SUPER+S` toggles the scratchpad.
* **Floating**: dialogs (xdg parent / X11 transient), fixed-size windows, and anything you float
  with `SUPER+V` or move with `SUPER`+drag. Tiled windows ignore client move/resize requests.
* **Decorations**: server side (none drawn); the focused window gets a border in the theme's
  accent colour (`wm.border`).
* **Focus** follows clicks and the model; an overlay of the shell (launcher, settings) keeps the
  keyboard while it is open and hands it back when it closes.

## Protocols

xdg-shell, xdg-decoration, wlr-layer-shell, xdg-activation, viewporter, fractional-scale,
presentation-time, linux-dmabuf (+ explicit sync where the GPU supports it), single-pixel-buffer,
fifo, commit-timing, data-device, primary-selection, wlr-data-control, pointer-constraints,
relative-pointer, pointer-gestures, tablet, text-input/input-method, virtual-keyboard,
keyboard-shortcuts-inhibit, xdg-foreign, security-context, drm-lease, ext-session-lock,
ext-idle-notify, idle-inhibit, xdg-system-bell, ext-foreign-toplevel-list, Xwayland shell and
keyboard grab.

## Control socket

`$EDEX_COMP_SOCKET` (default `$XDG_RUNTIME_DIR/edex-comp.sock`, mode 0600): newline-delimited JSON,
one reply per request; the schema is `comp-proto/src/proto.rs`.

| cmd | fields | reply / effect |
|---|---|---|
| `version` | | `{"version","mode"}` |
| `state` | | outputs, workspaces, windows, focus, keyboard layout, lock |
| `subscribe` | | `{"ok":true}`, then one event per line |
| `focus-window` | `id` | |
| `close`, `kill`, `minimize`, `toggle-maximize`, `toggle-fullscreen`, `toggle-float` | `id` (optional: the focused window) | |
| `restore` | `id`, `workspace` (optional) | |
| `move-to-workspace` | `id`, `workspace`, `follow` | |
| `focus-direction`, `move-direction` | `direction` | |
| `cycle-focus` | `reverse` | |
| `focus-workspace` | `workspace`: `{"kind":"id","id":N}` / `next` / `prev` / `empty` | |
| `toggle-scratch` | | |
| `set-app-area` | `output`, `tiled`, `maximized` (`{x,y,w,h}`) | |
| `exec` | `command`, `env` | `{"pid"}` |
| `lock`, `exit`, `reload`, `dpms` (`on`) | | |
| `screenshot` | `path`, `output`, `region` | PNG written |
| `binds` | | `{"binds":["SUPER+Q → close", …]}` |
| `services`, `restart-service` (`name`) | | session programs |
| `login` | `user`, `password`, `session` | greeter: start the session; session: verify for the lock screen |
| `power-off`, `reboot` | | greeter only |

Events: `window-opened`, `window-closed`, `window-changed`, `focus`, `workspace`, `monitors`,
`workspaces`, `keyboard-layout`, `bell`, `config-reloaded`, `locked`.

## Configuration

edex-comp reads `~/.config/edex-de/config.toml` (sections `wm`, `input`, `display`, `power`,
`appearance.theme`) and reloads it when it changes. The built-in key bindings are listed in the
README; `[[wm.binds]]` entries are applied over them.
