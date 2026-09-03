#!/usr/bin/env bash
# The version, from the one place it is written down.
#
#   packaging/version.sh            prints it
#   packaging/version.sh --check    fails if a packaging file disagrees
#
# Cargo.toml is the source of truth. The RPM spec and the PKGBUILD have to
# repeat it — neither format can read a value out of another file — so this
# checks that they still agree, because a package labelled with the wrong
# version is not something anyone notices until it is installed.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

if [ "${1:-}" != "--check" ]; then
    echo "$version"
    exit 0
fi

status=0

spec="$(sed -n 's/^Version: *\(.*\)$/\1/p' "$root/packaging/linux/gitdruid.spec" | head -1)"
pkgbuild="$(sed -n 's/^pkgver=\(.*\)$/\1/p' "$root/packaging/linux/PKGBUILD" | head -1)"

for pair in "gitdruid.spec:$spec" "PKGBUILD:$pkgbuild"; do
    file="${pair%%:*}"
    found="${pair#*:}"

    if [ "$found" != "$version" ]; then
        echo "$file says $found, Cargo.toml says $version" >&2
        status=1
    fi
done

exit $status
