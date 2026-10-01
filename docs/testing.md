# Testing

| Level | Command | Covers |
|---|---|---|
| Unit | `cargo test --workspace` | window model (tiling, maximize, minimize, workspaces, scratchpad), key binding parsing, control-socket round trips and event diffs, `CompState` from events, app areas from the shell layout, parsers for wpctl, `svc`, `/dev/bluetooth`, wg-quick and evdev, the NetworkManager mapping against a fake, sysfs backlight, SHA-512 crypt and account edits (`edex-auth`), config round trips, layout snapshots (insta), terminal VT behaviour on a real PTY, notification server on a private D-Bus |
| Lint | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` | |
| Nested | `cargo run -p edex-comp -- --nested` | the compositor, the shell and apps in a window of a Linux desktop session, by hand |
| RustOS | QEMU or hardware with the RustOS desktop milestones | the `edex` service: login, session, lock, settings backends |

CI (`.github/workflows/ci.yml`) runs the unit and lint levels on a Linux host. There are no
automated runtime tests yet: they need RustOS's graphics and desktop milestones (see
[rustos.md](rustos.md)); RustOS's `docs/DESKTOP.md` describes the scenario planned for then.
