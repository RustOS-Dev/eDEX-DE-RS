# eDEX-DE

[![CI](https://github.com/RustOS-Dev/eDEX-DE-RS/actions/workflows/ci.yml/badge.svg)](https://github.com/RustOS-Dev/eDEX-DE-RS/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-cyan.svg)](LICENSE)

**eDEX-DE** is the desktop of [RustOS](https://github.com/RustOS-Dev/RustOS): a sci-fi shell in the
style of [eDEX-UI](https://github.com/GitSquared/edex-ui), written in Rust, drawn with wgpu, with its
own Wayland compositor. It draws everything around your applications (the terminal, file browser,
system dashboard, launcher, settings, privacy panel, notifications, power menu, login and lock
screens), and the compositor, **edex-comp**, tiles applications into the shell's centre panel next
to its terminal tabs.

eDEX-DE targets RustOS. The full session runs there: the login screen, the shell and the lock
screen on edex-comp, on DRM/KMS (M35), the desktop kernel features (M36), the Wayland stack (M37)
and Mesa (M41), with GLES through GBM/EGL, or Mesa's software rasterizer when there is no GPU
driver. The desktop services of M42 (seatd, D-Bus, PipeWire, UPower, `rustos-nmd`) light up the
remaining panels. [docs/rustos.md](docs/rustos.md) lists exactly what it needs from RustOS.

## What you get

| Part | Implementation |
|---|---|
| Compositor | `edex-comp`, built on Smithay: DRM/KMS with GBM/EGL (Mesa, including its software rasterizer), or pixman on dumb buffers without Mesa; libinput, libseat, Xwayland. Tiles windows into the shell's centre panel, with workspaces per output, maximize, fullscreen, floating, minimize-to-tab and a scratchpad |
| Terminal | Multi-tab terminal on `alacritty_terminal` (alt screen, scroll regions, mouse reporting, bracketed paste, OSC 52, selection, scrollback), running RustOS's `sh` |
| Files | Clickable file browser with breadcrumbs, dotfiles toggle, open-in-terminal, `xdg-open` |
| Dashboard | CPU per core, memory, network sparklines, disks, processes; privacy indicators for Tor, VPN, WireGuard, microphone and camera |
| Keyboard | Optional on-screen hex keyboard for touchscreens (Settings → Appearance, or `Ctrl+Shift+K`) |
| Launcher | Fuzzy search over XDG desktop entries with launch history (tap `SUPER`, or `SUPER+Space`) |
| Settings | Appearance, display, input, audio (PipeWire/wpctl), network (NetworkManager D-Bus, served by `rustos-nmd`), Bluetooth (`/dev/bluetooth`), power (UPower, login1), users (`edex-auth`), notifications, services (RustOS `svc` and the session programs), window manager, terminal, about |
| Privacy | Tor (SOCKS mode, bootstrap and circuit state, NEWNYM, obfs4/snowflake bridges), WireGuard tunnels, Tailscale when installed, DNS check |
| Notifications | eDEX-DE is the `org.freedesktop.Notifications` server: toasts, actions, history, do-not-disturb, per-app mute |
| Login and lock | `edex-comp --greeter` runs `edex-greeter`; passwords are checked by `edex-auth` against RustOS's shadow file. The same greeter is the ext-session-lock lock screen |
| Config | `~/.config/edex-de/config.toml`, read by the shell and edex-comp and reloaded live |
| Themes | Tron, Matrix, Amber, Cyborg, Blade, Apollo, Interstellar, Horizon, Navy, Nord, Red, Purple; add your own in `~/.config/edex-de/themes` |
| IPC | `edex-de ipc …` drives the shell; `edex-comp msg|state|screenshot` drives the compositor |

## Install on RustOS

RustOS builds eDEX-DE from source in its ports tree and installs it under `/usr/local`, like its
Wayland stack (RustOS's `docs/DESKTOP.md`):

```bash
# in a RustOS checkout
git submodule update --init third_party/musl
rustup target add x86_64-unknown-linux-musl
tools/install-port.sh --initramfs weston       # Wayland, libinput, seatd, pixman, Mesa, ...
tools/install-port.sh --initramfs libunwind    # libgcc_s.so.1 for dynamically linked Rust
tools/install-port.sh --initramfs edex-de      # EDEX_SRC=$HOME/eDEX-DE-RS for a local checkout
tools/install-port.sh --initramfs jetbrains-mono-nerd
cargo build --features linux-drivers           # DRM (bochs, virtio-gpu, simpledrm) and input
./write_to_drive.sh --drive /dev/sdX
```

The port builds every binary with its default features: `edex-comp` renders with GLES through
Mesa's GBM/EGL (softpipe when there is no GPU driver) and falls back to pixman on DRM dumb buffers
without them (`EDEX_RENDERER=pixman` forces it); the shell and the greeter draw with wgpu, on
Vulkan or GLES. The desktop is turned on with the `edex` service (tty1, every boot):

```sh
svc enable edex && svc start edex
```

To try the compositor alone from a console, with a client of your choice:

```sh
mkdir -p /tmp/xdg; chmod 700 /tmp/xdg
export XDG_RUNTIME_DIR=/tmp/xdg LIBSEAT_BACKEND=builtin
edex-comp run --run weston-terminal &
```

The service runs `edex-comp --greeter` as root on tty1. After you log in, edex-comp starts your
session as you: the session D-Bus, PipeWire and WirePlumber, the portals and the shell, and restarts
any of them that crash.

## Develop

eDEX-DE builds and tests on any Linux host. The RustOS target is `x86_64-unknown-linux-musl`,
linked dynamically against the RustOS ports ([.cargo/config.toml](.cargo/config.toml)).

```bash
sudo apt install libxkbcommon-dev libwayland-dev libvulkan-dev libdbus-1-dev libegl-dev \
    libgbm-dev libdrm-dev libinput-dev libseat-dev libudev-dev libpixman-1-dev pkg-config
cargo build --workspace
cargo test --workspace

# Nested inside another Wayland or X11 session:
EDEX_SHARE_DIR=$PWD/share cargo run -p edex-comp -- run --nested
# …with only a terminal instead of the full session:
cargo run -p edex-comp -- run --nested --run foot
# The greeter without a compositor:
cargo run -p edex-greeter -- --demo
# edex-comp without Mesa (pixman only, no GBM/EGL; no nested mode):
cargo build -p edex-comp --no-default-features
```

## How a session starts

```
RustOS init → svc → service "edex" on tty1: edex-comp --greeter (root)
   edex-greeter (a Wayland client of edex-comp) → Login request
   edex-comp → edex-auth check USER (password on stdin)
   edex-comp opens /run/user/UID/{wayland-N, edex-comp.sock}, loads ~/.config/edex-de/config.toml,
   starts Xwayland, then as the user: dbus-daemon --session, pipewire, wireplumber,
   xdg-desktop-portal(-wlr), cliphist, edex-de run
Lock: SUPER+ALT+L, idle timeout or lid → edex-greeter --lock (ext-session-lock)
Log out: the power menu → edex-comp exits → svc starts the greeter again
```

## Windows and the centre tab strip

Applications tile into the centre panel, below its tab strip, which always stays visible. The strip
holds the terminal tabs, a tab for every window on the current workspace and one for every
minimized window. The focused window gets three controls at the right end of the strip:

* **↓** minimizes it into a tab (click the tab to bring it back),
* **□** maximizes it: it takes the full width while the side panels step aside; click again to
  restore,
* **×** closes it.

Clicking a terminal tab (or `+`) while apps cover the terminal minimizes them into tabs and shows
the terminal. Middle-click a window tab to close it. The shell tells edex-comp the size of the
centre panel whenever its layout changes, so resizing the side panels re-tiles the windows.

## Keyboard shortcuts

edex-comp's bindings. Add or override them in `config.toml` (`[[wm.binds]]`, see below); Settings →
Window manager lists the active set.

| Keys | Action |
|---|---|
| `SUPER` (tap) or `SUPER+Space` | Launcher |
| `SUPER+,` | Settings |
| `SUPER+P` | Privacy panel |
| `SUPER+N` | Notification history |
| `SUPER+Escape` | Power menu |
| `SUPER+Return` / `SUPER+F1` | Focus the eDEX terminal / file panel |
| `SUPER+Shift+Return` | foot |
| `SUPER+Q`, `SUPER+Shift+Q` | Close, kill the focused window |
| `SUPER+V`, `SUPER+C` | Float, centre the focused window |
| `SUPER+M` | Minimize the focused window into a tab |
| `SUPER+F` | Maximize: full width, side panels hidden, top bar and window controls stay |
| `SUPER+Shift+F` | True fullscreen over everything |
| `SUPER+Ctrl+F` | Hide / show the side panels for all apps |
| `SUPER+H/K/L`, arrows | Focus left / up / right / … |
| `SUPER+Shift+H/J/K/L`, arrows | Swap the focused window with its neighbour |
| `SUPER+Tab`, `SUPER+Shift+Tab` | Cycle focus |
| `SUPER+1..0`, `SUPER+Shift+1..0`, `SUPER+Ctrl+1..0` | Switch to / move to / move with the window to a workspace |
| `SUPER+[`, `SUPER+]`, `SUPER`+wheel | Previous / next workspace |
| `SUPER+S` | Scratchpad |
| `SUPER`+drag, `SUPER`+right-drag | Move, resize a window (it floats) |
| `SUPER+Alt+L` | Lock |
| `SUPER+Shift+R` | Reload the configuration |
| `Print`, `SUPER+Shift+S` | Screenshot of the output (to `~/Pictures/Screenshots`), of a region (clipboard) |
| `Ctrl+Alt+F1..F12` | Virtual terminals |
| Media keys | Volume, brightness and playback (also on the lock screen) |

Inside the shell:

| Keys | Action |
|---|---|
| `Ctrl+Shift+T` / `Ctrl+Shift+W` | New / close terminal tab |
| `Alt+1..9`, `Ctrl+Tab` | Switch tab |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste |
| `Shift+PgUp` / `Shift+PgDn` | Scrollback |
| `Ctrl+Shift+F` | Toggle focus between terminal and file panel |
| `Ctrl+Shift+K` | Show / hide the on-screen keyboard |
| Overlays: `Esc` closes, `Tab`/arrows move, `Enter` activates, `Ctrl+PgUp/PgDn` switch tabs |

## Configuration

`~/.config/edex-de/config.toml` is created on first start. The shell and edex-comp both watch it and
apply changes immediately. Every key is optional:

```toml
[appearance]
theme = "tron"            # any file in /usr/share/edex-de/themes or ~/.config/edex-de/themes
font = "JetBrainsMono Nerd Font"
font_size = 14.0
border_glow = 0.8
scanlines = true
animations = true
keyboard_visible = false  # on-screen hex keyboard, for touchscreens
boot_animation = true

[layout]
fs_split = 0.20           # file panel width
sysinfo_split = 0.78      # where the system panel starts
reserve_side_panels = true

[terminal]
shell = ""                # empty = your login shell
scrollback = 10000
font_size = 13.0
cursor = "block"          # block | underline | beam
cursor_blink = true
bell = "visual"           # visual | audible | none
osc52_read = false

[launcher]
terminal_command = "foot"

[notifications]
dnd = false
timeout_ms = 5000
max_visible = 4
muted_apps = []

[wm]                      # edex-comp
gaps_in = 4
gaps_out = 8
border = 2                # focus border in the theme's accent colour
layout = "dwindle"        # dwindle | master
workspaces = 9

[[wm.binds]]              # added to (or replacing) the built-in bindings
keys = "SUPER+SHIFT+Return"
action = "exec kitty"     # exec CMD | shell IPC-ARGS | close | kill | minimize | maximize |
                          # fullscreen | float | center | focus DIR | move DIR | cycle next|prev |
                          # workspace N|next|prev|empty|scratch | movetoworkspace N [follow] |
                          # scratch | lock | exit | reload | screenshot output|region | vt N | none

[input]
kb_layout = "us"
kb_variant = ""
kb_options = ""
repeat_rate = 30
repeat_delay = 300
natural_scroll = true
tap_to_click = true
sensitivity = 0.0

[display]
night_light = false       # CRTC gamma
night_temp = 4000
[[display.monitors]]
name = ""                 # empty = every output without a rule of its own
mode = "preferred"        # or 2560x1440@144
position = "auto"         # or 1920x0
scale = 1.0
transform = 0             # 0..3 rotations, 4..7 flipped
disabled = false

[power]                   # edex-comp's idle handling
lock_after = 600          # seconds, 0 = never
dpms_after = 900
lid_close = "suspend"     # suspend (panel off, lock) | lock | poweroff | ignore
lock_on_sleep = true

[privacy]
tor_mode_on_login = false
```

## Layout of the repository

| Path | What |
|---|---|
| `edex-comp/` | The compositor (Smithay; started from Smithay's anvil, MIT, licence kept) |
| `comp-proto/` | edex-comp's control protocol; `comp/` is the shell's client and state model |
| `edex-de/` | The shell application |
| `edex-greeter/` | Login and lock screen |
| `edex-auth/` | Password checks and account edits (no PAM on RustOS) |
| `platform/`, `renderer/`, `ui/`, `terminal/` | Wayland client, wgpu renderer, scene/layout, terminal |
| `system/`, `sysmon/`, `notifications/`, `launcher/`, `settings/`, `ipc/` | Backends, monitoring, notification server, launcher, config, shell IPC |
| `share/libexec/` | Helpers installed to `/usr/local/libexec/edex-de` (Tor mode and bridges) |
| `themes/`, `assets/`, `packaging/` | Themes, artwork, greeter config and desktop entries |

See [docs/architecture.md](docs/architecture.md), [docs/compositor.md](docs/compositor.md),
[docs/ipc.md](docs/ipc.md), [docs/rustos.md](docs/rustos.md) and [docs/testing.md](docs/testing.md).

## License

GPL-3.0. `edex-comp` contains code from Smithay's anvil (MIT); see `edex-comp/LICENSE-anvil.txt`.
