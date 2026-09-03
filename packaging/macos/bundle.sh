#!/usr/bin/env bash
# Builds gitDruid.app, and optionally a disk image to hand someone.
#
# Usage: packaging/macos/bundle.sh [--dmg] [--universal]
#
# The bundle is ad-hoc signed. That is not the same as a Developer ID
# signature: it stops macOS calling the app damaged on Apple Silicon, but
# anyone who did not build it themselves will still have to allow it the first
# time. See packaging/README.md for what notarising would take.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

name="gitDruid"
binary="git-druid"
identifier="io.digitalsynapse.gitdruid"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

dmg=false
universal=false

for argument in "$@"; do
    case "$argument" in
        --dmg) dmg=true ;;
        --universal) universal=true ;;
        *) echo "unknown option: $argument" >&2; exit 2 ;;
    esac
done

out="$root/target/bundle"
app="$out/$name.app"

echo "==> building $name $version"

if $universal; then
    # A universal binary needs both targets present; without them, say so
    # rather than quietly shipping one architecture.
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        if ! rustup target list --installed | grep -qx "$target"; then
            echo "missing target $target — run: rustup target add $target" >&2
            exit 1
        fi
    done

    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin

    mkdir -p "$out"
    lipo -create -output "$out/$binary" \
        "target/aarch64-apple-darwin/release/$binary" \
        "target/x86_64-apple-darwin/release/$binary"

    built="$out/$binary"
else
    cargo build --release
    built="target/release/$binary"
fi

echo "==> assembling $app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$built" "$app/Contents/MacOS/$binary"
chmod +x "$app/Contents/MacOS/$binary"

# The icon set, from the sizes packaging/icon.py renders.
iconset="$out/$name.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"

python3 packaging/icon.py packaging/icons >/dev/null

cp packaging/icons/16.png   "$iconset/icon_16x16.png"
cp packaging/icons/32.png   "$iconset/icon_16x16@2x.png"
cp packaging/icons/32.png   "$iconset/icon_32x32.png"
cp packaging/icons/64.png   "$iconset/icon_32x32@2x.png"
cp packaging/icons/128.png  "$iconset/icon_128x128.png"
cp packaging/icons/256.png  "$iconset/icon_128x128@2x.png"
cp packaging/icons/256.png  "$iconset/icon_256x256.png"
cp packaging/icons/512.png  "$iconset/icon_256x256@2x.png"
cp packaging/icons/512.png  "$iconset/icon_512x512.png"
cp packaging/icons/1024.png "$iconset/icon_512x512@2x.png"

iconutil --convert icns --output "$app/Contents/Resources/$name.icns" "$iconset"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundleDisplayName</key><string>$name</string>
    <key>CFBundleIdentifier</key><string>$identifier</string>
    <key>CFBundleExecutable</key><string>$binary</string>
    <key>CFBundleIconFile</key><string>$name</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc, so the bundle at least has a valid signature over its own contents.
codesign --force --deep --sign - "$app"
codesign --verify --strict "$app" && echo "==> signature verifies"

echo "==> $app"

if $dmg; then
    image="$out/$name-$version.dmg"
    staging="$out/dmg"

    rm -rf "$staging" "$image"
    mkdir -p "$staging"
    cp -R "$app" "$staging/"
    ln -s /Applications "$staging/Applications"

    hdiutil create -volname "$name" -srcfolder "$staging" -ov -format UDZO "$image" >/dev/null
    rm -rf "$staging"

    echo "==> $image"
fi
