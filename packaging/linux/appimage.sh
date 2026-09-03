#!/usr/bin/env bash
# Builds a single-file AppImage: one executable that carries its own
# dependencies and needs no installation at all.
#
# Usage: packaging/linux/appimage.sh
#
# Needs appimagetool on PATH:
#   wget https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
#   chmod +x appimagetool-*.AppImage && sudo mv appimagetool-*.AppImage /usr/local/bin/appimagetool
#
# Build on the oldest distribution you support — an AppImage carries its own
# libraries but still links the host's glibc.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

version="$(packaging/version.sh)"
packaging/version.sh --check
arch="$(uname -m)"

appdir="target/appimage/gitDruid.AppDir"

if ! command -v appimagetool >/dev/null 2>&1; then
    echo "appimagetool is not on PATH — see the comment at the top of this script" >&2
    exit 1
fi

cargo build --release --features bundled

echo "==> assembling $appdir"

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications"

cp target/release/git-druid "$appdir/usr/bin/git-druid"

python3 packaging/icon.py packaging/icons >/dev/null

for size in 16 32 48 64 128 256 512; do
    dir="$appdir/usr/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    cp "packaging/icons/$size.png" "$dir/gitdruid.png"
done

# appimagetool wants the icon and the desktop file at the AppDir root too.
cp packaging/icons/256.png "$appdir/gitdruid.png"
cp packaging/gitdruid.desktop "$appdir/gitdruid.desktop"
cp packaging/gitdruid.desktop "$appdir/usr/share/applications/gitdruid.desktop"

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
# Passes the arguments through, so `gitDruid.AppImage /path/to/repo` opens it.
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/git-druid" "$@"
RUN

chmod +x "$appdir/AppRun"

ARCH="$arch" appimagetool "$appdir" "target/appimage/gitDruid-$version-$arch.AppImage"

echo "==> target/appimage/gitDruid-$version-$arch.AppImage"
echo
echo "It runs as it is. To get a menu entry without root, either install"
echo "appimaged, or use the portable tarball, which ships an installer."
