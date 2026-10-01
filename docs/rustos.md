# eDEX-DE on RustOS: what it needs

eDEX-DE is the desktop of RustOS. This page is the contract between the two: the RustOS features
eDEX-DE relies on, where they come from in the RustOS roadmap (`docs/ROADMAP-ROUND4.md` in RustOS),
and how eDEX-DE behaves when one is missing.

## Building

* Target `x86_64-unknown-linux-musl`, dynamically linked (`-C target-feature=-crt-static`) against
  the RustOS ports, baseline x86-64 (the RustOS kernel saves FPU state with FXSAVE: no AVX).
* The RustOS port recipe (`ports/edex-de/build.sh` in RustOS) builds a pinned commit (or your
  checkout with `EDEX_SRC=…`), links with `tools/rustos-cc`, finds the desktop libraries through
  `tools/cross/pkg-config` (M37), and installs (RustOS's `RUSTOS_DESKTOP=1` build puts the desktop
  ports of `ports/desktop.list` into the image):

| From | To |
|---|---|
| `edex-comp`, `edex-de`, `edex-greeter`, `edex-auth` | `/usr/bin` |
| `themes/*.toml` | `/usr/share/edex-de/themes` |
| `share/libexec/*` | `/usr/libexec/edex-de` |
| `packaging/greeter/greeter.toml` | `/etc/edex-greeter/greeter.toml` |
| `packaging/applications/*.desktop` | `/usr/share/applications` |

Other RustOS ports in `ports/desktop.list`: `jetbrains-mono-nerd` (the UI font), `tor`,
`tor-pt` and `wireguard-tools` (`wg`, for WireGuard keys).

## Kernel and system (RustOS milestones)

| Need | Used by | Milestone |
|---|---|---|
| DRM/KMS (`/dev/dri/card*`, `renderD*`), atomic modesetting, gamma LUT, dma-buf | edex-comp output, night light | M35, M38–M40 |
| Mesa EGL/GBM (GL ES 2+; `kms_swrast` on efidrm without a GPU driver), Vulkan for wgpu | edex-comp rendering, shell rendering | M41 (M37 softpipe) |
| `AF_UNIX` named sockets with `SCM_RIGHTS` and `SO_PEERCRED` | Wayland, D-Bus, the control and IPC sockets | M36 |
| `memfd_create` + seals, `MAP_SHARED` | Wayland shm buffers, keymaps | M36 |
| `inotify` | live config reload (`notify` crate) | M36 |
| uevents (`NETLINK_KOBJECT_UEVENT`) + libudev-zero | hot-plugged outputs and input devices | M36, M37 |
| VT switching (`VT_SETMODE`/`VT_PROCESS`, `KDSKBMODE K_OFF`), DRM master handover, `EVIOCREVOKE` | libseat/seatd, `Ctrl+Alt+Fn` | M36 |
| evdev + libinput, libxkbcommon + xkeyboard-config | input | M37 |
| seatd | device access for edex-comp | M42 |
| wayland, wayland-protocols, pixman | edex-comp, all clients | M37 |
| Xwayland (optional) | X11 applications | M42 |
| D-Bus system and session buses | notifications, UPower, login1, NetworkManager API | M37, M42 |
| PipeWire + WirePlumber (`wpctl`, `pw-play`) | audio settings, volume keys, bell | M42 |
| UPower over `/sys/class/power_supply` | battery | M42 |
| login1 subset (`CanReboot`, `Reboot`, `PowerOff`, …) | power menu | M42 |
| `/sys/class/backlight/*/{brightness,max_brightness,type}`, writable by the session | brightness | M38–M40 (DRM panels), M33 |
| CPU-time accounting in `/proc/stat` (per-CPU lines), `/proc/[pid]/stat` utime/stime, `/proc/loadavg`, `/proc/uptime` idle | the system dashboard | RustOS M43 (this port) |
| the `svc` service manager | Services panel, Tor, the `edex` service | RustOS M43 (this port) |
| Linux signal frames (`siginfo`, `ucontext`, `sigaltstack`), per-thread signal masks | Rust's stack-overflow handler, the Go pluggable transports (lyrebird, snowflake) | RustOS M43 (this port) |
| `libgcc_s.so.1` (the `libunwind` port) | every eDEX binary (Rust's std, dynamically linked for musl) | RustOS M43 (this port) |

## NetworkManager D-Bus subset (`rustos-nmd`)

`system/src/network.rs` uses only these calls; `rustos-nmd` (M42) must serve them:

| Object | Members |
|---|---|
| `org.freedesktop.NetworkManager` (`/org/freedesktop/NetworkManager`) | properties `Connectivity` (u), `WirelessEnabled` (b, writable), `WirelessHardwareEnabled` (b), `ActiveConnections` (ao); methods `GetDevices() → ao`, `ActivateConnection(o, o, o) → o`, `AddAndActivateConnection(a{sa{sv}}, o, o) → (o, o)`, `DeactivateConnection(o)` |
| `…NetworkManager.Settings` (`/org/freedesktop/NetworkManager/Settings`) | `ListConnections() → ao`, `AddConnection(a{sa{sv}}) → o` |
| `…Settings.Connection` | `GetSettings() → a{sa{sv}}` (`connection.id`, `connection.type`), `GetSecrets(s) → a{sa{sv}}` (`wireguard.private-key`), `Delete()` |
| `…Connection.Active` | `Connection` (o), `Devices` (ao) |
| `…Device` | `Interface` (s), `DeviceType` (u: 1 ethernet, 2 Wi-Fi) |
| `…Device.Wireless` | `GetAllAccessPoints() → ao`, `RequestScan(a{sv})`, `ActiveAccessPoint` (o) |
| `…AccessPoint` | `Ssid` (ay), `Strength` (y), `HwAddress` (s), `Flags`, `WpaFlags`, `RsnFlags` (u) |

Connection types used: `802-11-wireless` (with `802-11-wireless-security.key-mgmt` `wpa-psk` or
`sae`), `802-3-ethernet`, and `wireguard` (`wireguard.private-key`, `wireguard.peers` as
`aa{sv}` with `public-key`, `endpoint`, `allowed-ips`, `persistent-keepalive`; `ipv4/ipv6.method`
and `address-data`). WireGuard needs the kernel driver (LinuxKPI `wireguard` group) and `wg`
(wireguard-tools) for key generation.

## Bluetooth

RustOS's native stack through `/dev/bluetooth`, as the `bt` tool uses it (write one command, read
lines until EOF, answer `? ` questions): `status`, `list`, `devices`, `scan N`, `pair ADDR`,
`connect ADDR`, `disconnect ADDR`, `remove ADDR`, `power on|off`. No bluez.

## `svc`

The Services panel and the Tor helpers call:

```text
svc list --json              [{"name","description","enabled","state","pid","tty"}, …]
svc status NAME --json       one such object
svc start|stop|restart|enable|disable NAME
```

`state` is `running`, `stopped`, `starting` or `failed`. Exit status 0 is success, 1 an error
(message on stderr), 3 an unknown service.

Services eDEX-DE expects (shipped disabled by RustOS until their programs are installed):

| Service | Runs |
|---|---|
| `edex` | `edex-comp --greeter` on tty1 |
| `seatd`, `dbus`, `upower`, `rustos-nmd` | the desktop services |
| `tor` | `tor -f /storage/etc/tor/torrc` (or `/etc/tor/torrc`), switched by `edex-tor-mode`; bridges through the `tor-pt` port (lyrebird, snowflake-client) |

## Accounts

`edex-auth` works on the files RustOS's `login` and `passwd` use: `/storage/etc/{passwd,shadow,group}`
first, then `/etc`, SHA-512 crypt (`$6$`), an empty hash for "no password" and `!`/`*` for a
locked account; edits are written to `/storage/etc`. RustOS currently runs everything as uid 0; once
it has real users, install `edex-auth` set-uid root: it lets callers change only their own account
and requires root for lock and unlock.

The same goes for the system changes the shell makes directly: `svc start|stop|enable|disable`
(init's control FIFO is root-only), `/usr/libexec/edex-de/edex-tor-*` (they write
`/storage/etc/tor` and drive `svc`) and the backlight. With real users these need a small
privilege broker (the role polkit plays on Linux); until then the shell's session runs as root.

## Without the full stack

* No GPU driver: edex-comp renders with Mesa's software rasterizer on efidrm.
* No Xwayland: X11 applications do not start; everything else works.
* A missing service (UPower, `rustos-nmd`, PipeWire, Tor, Tailscale, fprintd, power-profiles-daemon):
  the matching panel says "unavailable" instead of showing stale values.
* No packet filter: the Privacy panel says so; Tor runs in SOCKS mode only.
* No suspend: the lid action "suspend" turns the internal panel off and locks the session.
