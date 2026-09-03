#!/usr/bin/env bash
# Builds a tarball that runs from wherever it is unpacked, and can add itself
# to the desktop menu without root.
#
# This is the answer for a machine where installing an RPM is not allowed:
# unpack it anywhere, run ./git-druid, and optionally ./install.sh to get a
# menu entry and an icon under ~/.local.
#
# Usage: packaging/linux/portable.sh
#
# Build this on the OLDEST distribution you intend to support. A binary links
# against the glibc it was built with and will not run on anything older, so
# one built on Ubuntu 25.04 will not start on RHEL 9. The reverse works.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

binary="git-druid"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
arch="$(uname -m)"

stage="target/portable/gitdruid-$version-linux-$arch"

echo "==> building (with libgit2 and OpenSSL vendored, so the binary does not"
echo "    depend on the versions this distribution happens to ship)"

cargo build --release --features bundled

echo "==> assembling $stage"

rm -rf "$stage"
mkdir -p "$stage/icons"

cp "target/release/$binary" "$stage/$binary"
chmod +x "$stage/$binary"

python3 packaging/icon.py packaging/icons >/dev/null
for size in 16 32 48 64 128 256 512; do
    cp "packaging/icons/$size.png" "$stage/icons/$size.png"
done

cp packaging/gitdruid.desktop "$stage/gitdruid.desktop"
cp README.md "$stage/README.md"

cat > "$stage/install.sh" <<'INSTALL'
#!/usr/bin/env sh
# Adds gitDruid to this user's desktop menu. No root, nothing outside $HOME.
#
# Everything lands under ~/.local, which every desktop environment reads
# without being told to. Run ./uninstall.sh to take it all back out.

set -eu

here="$(cd "$(dirname "$0")" && pwd)"

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

mkdir -p "$bin" "$data/applications"

cp "$here/git-druid" "$bin/git-druid"
chmod +x "$bin/git-druid"

for size in 16 32 48 64 128 256 512; do
    dir="$data/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    cp "$here/icons/$size.png" "$dir/gitdruid.png"
done

# The launcher names the binary by its full path: ~/.local/bin is not on
# everyone's PATH, and a menu entry that only works for some people is worse
# than one that works for all of them.
sed "s|^Exec=git-druid|Exec=$bin/git-druid|" "$here/gitdruid.desktop" \
    > "$data/applications/gitdruid.desktop"

chmod 644 "$data/applications/gitdruid.desktop"

# Caches, where the desktop bothers with them. Missing tools are not an error:
# most environments notice a new .desktop file on their own.
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true

echo "Installed to $bin/git-druid"
echo "Menu entry:  $data/applications/gitdruid.desktop"

case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo
       echo "Note: $bin is not on your PATH, so 'git-druid' will not work from"
       echo "a shell until you add it. The menu entry works either way." ;;
esac
INSTALL

cat > "$stage/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env sh
set -eu

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

rm -f "$bin/git-druid"
rm -f "$data/applications/gitdruid.desktop"

for size in 16 32 48 64 128 256 512; do
    rm -f "$data/icons/hicolor/${size}x${size}/apps/gitdruid.png"
done

command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$data/applications" || true

echo "Removed. Settings in ~/.config/gitDruid were left alone."
UNINSTALL

chmod +x "$stage/install.sh" "$stage/uninstall.sh"

tar -czf "$stage.tar.gz" -C "$(dirname "$stage")" "$(basename "$stage")"

echo "==> $stage.tar.gz"
