# Contributing

## Setup

```bash
sudo pacman -S --needed rust libxkbcommon wayland vulkan-icd-loader sway foot grim librsvg
cargo build --workspace
cargo test --workspace
scripts/smoke-sway.sh            # headless end-to-end run with screenshots in target/smoke
```

Run the shell inside a nested Hyprland with `EDEX_SHARE_DIR=$PWD/share cargo run -p edex-de -- run`.

## Rules

* `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`
  must pass; CI also runs the smoke tests and builds the Arch package.
* Every visible feature needs a real backend: no placeholder values, no stubbed panels. If a backend is
  unavailable at runtime the UI must say so (see `system::*State::available`).
* New backends go in the `system` crate behind `CommandRunner` so they can be tested with `FakeRunner`.
* UI drawing lives in `ui` (no GPU code); rendering in `renderer`; wiring in `edex-de`.
* Keep the Hyprland Lua files (`share/hypr`) valid for Hyprland 0.55+: check with `Hyprland --verify-config`.
* Update `CHANGELOG.md` for user-visible changes.

## Releasing

1. Bump `version` in `Cargo.toml`, `packaging/aur/PKGBUILD`, `packaging/rpm/edex-de.spec` and
   `packaging/debian/changelog`; add a `CHANGELOG.md` section.
2. Tag `vX.Y.Z` (pre-releases: `vX.Y.Z-rc.N`). The release workflow builds the tarball, .deb and .rpm,
   uploads them with `SHA256SUMS`, and publishes the PKGBUILD to the AUR for final releases when the
   `AUR_SSH_PRIVATE_KEY` secret exists.
