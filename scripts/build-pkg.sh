#!/usr/bin/env bash
# Build the Arch package from the current tree (no network fetch of the source tarball).
# Run as a normal user inside an Arch environment: scripts/build-pkg.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
V=$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)
cd "$ROOT/packaging/aur"
rm -rf src pkg "edex-de-$V.tar.gz"
git -C "$ROOT" archive --format=tar.gz --prefix="eDEX-DE-$V/" HEAD > "edex-de-$V.tar.gz"
sed -e "s/^pkgver=.*/pkgver=$V/" -e "s|^source=.*|source=(\"edex-de-$V.tar.gz\")|" PKGBUILD > PKGBUILD.local
sudo pacman -S --needed --noconfirm --asdeps $(source PKGBUILD.local; echo "${makedepends[@]}") >/dev/null
makepkg -p PKGBUILD.local --syncdeps --noconfirm --skipchecksums -f
rm -f PKGBUILD.local
ls -1 ./*.pkg.tar.zst
