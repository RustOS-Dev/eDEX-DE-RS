# Changelog

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
