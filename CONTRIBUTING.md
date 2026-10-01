# Contributing

## Setup

On a Linux development host (Debian/Ubuntu package names):

```bash
sudo apt install libxkbcommon-dev libwayland-dev libvulkan-dev libdbus-1-dev libegl-dev \
    libgbm-dev libdrm-dev libinput-dev libseat-dev libudev-dev libpixman-1-dev pkg-config
cargo build --workspace
cargo test --workspace
EDEX_SHARE_DIR=$PWD/share cargo run -p edex-comp -- --nested     # compositor + shell in a window
```

To try it on RustOS, build the `edex-de` port from your checkout (see RustOS's `docs/DESKTOP.md`).

## Rules

* `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` must pass.
* eDEX-DE targets RustOS only. Anything it needs from the system is listed in
  [docs/rustos.md](docs/rustos.md); update that page when you start using a new file, command,
  D-Bus call or kernel feature.
* Every visible feature needs a real backend: no placeholder values, no stubbed panels. If a backend
  is unavailable at runtime the UI must say so (see `system::*State::available`).
* New backends go in the `system` crate behind `CommandRunner` (or a trait like `Nm` and
  `BtControl`) so they can be tested with fakes.
* Window management logic goes in `edex-comp/src/wm.rs` (pure, unit tested); Smithay wiring in the
  other modules. Control-socket changes go in `comp-proto` and [docs/compositor.md](docs/compositor.md).
* UI drawing lives in `ui` (no GPU code); rendering in `renderer`; wiring in `edex-de`.
* Update `CHANGELOG.md` for user-visible changes.

## Releasing

1. Bump `version` in `Cargo.toml` and add a `CHANGELOG.md` section.
2. Tag `vX.Y.Z`. The release workflow uploads the source tarball and `SHA256SUMS`.
3. Point RustOS's `ports/edex-de` recipe at the new tag.
