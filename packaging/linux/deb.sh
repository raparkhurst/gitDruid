#!/usr/bin/env bash
# Builds a .deb for Ubuntu 25.04 and anything else using dpkg.
#
# Usage: packaging/linux/deb.sh
#
# Assembles the tree by hand rather than pulling in debhelper: there is one
# binary, one desktop file and a handful of icons, and dpkg-deb can package
# that directly.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

version="$(packaging/version.sh)"
packaging/version.sh --check

case "$(uname -m)" in
    x86_64)  arch=amd64 ;;
    aarch64) arch=arm64 ;;
    *)       echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

stage="target/deb/gitdruid_${version}_${arch}"

cargo build --release --features bundled

rm -rf "$stage"
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/applications"

install -Dm755 target/release/git-druid "$stage/usr/bin/git-druid"

python3 packaging/icon.py packaging/icons >/dev/null

for size in 16 32 48 64 128 256 512; do
    install -Dm644 "packaging/icons/$size.png" \
        "$stage/usr/share/icons/hicolor/${size}x${size}/apps/gitdruid.png"
done

install -Dm644 packaging/gitdruid.desktop \
    "$stage/usr/share/applications/gitdruid.desktop"

cat > "$stage/DEBIAN/control" <<CONTROL
Package: gitdruid
Version: $version
Section: devel
Priority: optional
Architecture: $arch
Depends: libc6, libfontconfig1, libxkbcommon0
Recommends: mesa-vulkan-drivers
Maintainer: Robert Parkhurst <raparkhurst@digitalsynapse.io>
Description: A cross-platform git GUI focused on building good commits
 gitDruid shows what changed, stages it a hunk at a time, and writes the
 message. It draws the history of every ref as a coloured graph, manages
 branches and tags under a configurable workflow, and fetches, pulls and
 pushes. Several repositories can be open at once, one per tab.
CONTROL

dpkg-deb --root-owner-group --build "$stage" "target/deb/gitdruid_${version}_${arch}.deb"

echo "==> target/deb/gitdruid_${version}_${arch}.deb"
