# Testing

| Level | Command | Covers |
|---|---|---|
| Unit | `cargo test --workspace` | parsers for wpctl/nmcli/bluetoothctl/systemctl/tailscale/evdev, config round trips, layout snapshots (insta), terminal VT behaviour on a real PTY, notification server on a private D-Bus, greetd conversation against a fake socket |
| Headless | `scripts/smoke-sway.sh` | sway (headless, pixman) + llvmpipe: canvas and reservers configure, ≥3 frames, `notify-send` → toast, a `foot` window tiles inside the reserved slot, every overlay opens through IPC, scene dump, screenshots |
| Greeter | `scripts/smoke-greeter.sh` | greeter renders in demo mode |
| Package | `scripts/build-pkg.sh` | makepkg from the checkout in an Arch container (CI) |
| Real | nested Hyprland | `EDEX_SHARE_DIR=$PWD/share cargo run -p edex-de -- run`; check binds, tiling, settings and the privacy panel by hand. eDEX-OS runs the ISO in QEMU and asserts the shell's layer surfaces exist |

CI (`.github/workflows/ci.yml`) runs all of the above except the real-Hyprland session; screenshots are
uploaded as the `smoke-screenshots` artifact.
