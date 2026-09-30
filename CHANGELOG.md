# Changelog

## 3.1.0 — 2026-09-30

* **Input works.** Seats that already existed when the shell connected were ignored, so no keyboard
  or pointer was ever created: clicks and typing did nothing (the greeter shares the code). The
  terminal also has keyboard focus from login.
* **Windows key.** Tapping SUPER on its own opens the launcher; SUPER+Return switches to an empty
  workspace to show and focus the terminal when apps cover it.
* **Hyprland 0.56 dispatch.** Workspace buttons, Log out, launching apps and hypridle's screen
  off/on used the pre-0.55 string syntax and failed; they now use Lua dispatchers.
* **On-screen keyboard** is off by default: an opt-in setting for touchscreens (Settings →
  Appearance, or Ctrl+Shift+K).
* **Terminal** answers device-attribute, cursor-position, colour and size queries (fish no longer
  waits 10 s at startup).
* **Idle CPU.** Redraws only when something visibly changes; the border pulse is off on software
  renderers (llvmpipe idle CPU 108% → 14%).
* Apps launched outside Hyprland get their own systemd scope and survive a shell restart; the
  settings theme list no longer overlaps the next field.
* Release workflow: `ci.yml` is callable (`workflow_call`), so tag pushes run the release job
  (v3.0.0 was never published for this reason).

## 3.0.0 — 2026-09-30

First stable release of the Hyprland shell: everything in 3.0.0-rc.1, plus

* Hyprland config modules are namespaced and loaded with `require` from `/usr/share/edex-de/hypr`;
  unknown config keys removed (checked with `Hyprland --verify-config`).
* `build-pkg.sh` honours `PKGDEST`.
* Release workflow: the Debian build skips the apt Build-Depends check (Rust comes from rustup), package
  tests are not re-run after CI, and AUR publishing sees its deploy key.

Shipped in eDEX-OS 1.0.0; tested there in QEMU (live session, greeter, installed system). Not yet tested
on real GPUs.

## 3.0.0-rc.1 — 2026-09-28

Complete rewrite as a shell on Hyprland.

* The in-tree smithay compositor is gone; Hyprland (0.55+, Lua config) manages windows and eDEX-DE draws
  panels on the layer shell with reserver surfaces so applications tile into the terminal slot.
* New crates: `platform` (sctk client), `renderer` (wgpu instanced rects + glyphon text), `ui`
  (scene/layout/widgets/panels/overlays), `terminal` (alacritty_terminal), `hypr`, `ipc`, `settings`,
  `launcher`, `notifications` (D-Bus server), `sysmon`, `system` (backends), `edex-greeter`.
* Settings: 14 categories wired to wpctl, nmcli, bluetoothctl, upower/logind/power-profiles-daemon,
  brightnessctl, systemd, fprintd, hyprctl and hyprsunset.
* Privacy panel: Tor modes with control-port status, bridges, Tailscale, VPNs, DNS/firewall status.
* Notification server with toasts, actions, history, DND and per-app mute; OSD for volume/brightness.
* Config file with live reload and Hyprland/hypridle export; twelve themes.
* greetd greeter (`cage -s -- edex-greeter`) with session picker and power buttons.
* Packaging: PKGBUILD, Debian, RPM, greetd/polkit/tmpfiles files; CI with headless sway smoke test,
  Arch container package build; tagged releases with checksums.

## 2.0.8 and earlier

See the git history; these releases shipped the smithay-based compositor.
