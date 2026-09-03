# Packaging gitDruid

Four artifacts, because the situations are genuinely different:

| Situation | Build | Result |
|---|---|---|
| macOS | `packaging/macos/bundle.sh --dmg` | `gitDruid.app`, and a disk image |
| Linux, allowed to install packages | `packaging/linux/deb.sh`, `rpmbuild`, `makepkg` | `.deb`, `.rpm`, Arch package |
| **Linux, not allowed to install anything** | `packaging/linux/portable.sh` | a tarball that runs where it is unpacked and can add a menu entry under `~/.local` |
| Linux, one file and nothing else | `packaging/linux/appimage.sh` | a single `.AppImage` |

The icon is generated, not checked in: `packaging/icon.py` draws it with the
standard library alone, so no image tooling is needed to build a release. Every
script runs it. To change the icon, change the numbers at the top of `draw`.

## The unprivileged Linux case

This is the one with a constraint attached, so it gets the most care.

```sh
tar xzf gitdruid-0.0.5-linux-x86_64.tar.gz
cd gitdruid-0.0.5-linux-x86_64
./git-druid                 # runs, right there, installing nothing
./install.sh                # optional: menu entry and icon, no root
```

`install.sh` writes to `~/.local/bin`, `~/.local/share/applications` and
`~/.local/share/icons/hicolor`, which is where every desktop environment looks
without being told to. It rewrites the launcher's `Exec` line to the absolute
path of the installed binary, because `~/.local/bin` is not on everyone's
`PATH` and a menu entry that works for only some people is worse than one that
works for all of them. `./uninstall.sh` takes it all back out and leaves
`~/.config/gitDruid` alone.

### The AppImage, and the menu

An AppImage does not integrate itself. The format writes nothing outside the
file it is, on purpose, and the tools that do integrate one —
AppImageLauncher, `appimaged` — are packages someone has to install first,
which is the thing that is not allowed here.

So gitDruid does it itself, when asked. **Settings → Add to menu** writes a
launcher and icons under `~/.local/share`, pointing at whatever is actually
running: the AppImage when the `APPIMAGE` environment variable is set, and the
executable otherwise. The icons are carried inside the binary, so a single file
really is a single file. The same button removes it again, and settings in
`~/.config/gitDruid` are left alone either way.

It is offered rather than done. Writing to someone's applications menu without
asking is not a thing an application should decide for itself.

## The glibc rule

**Build on the oldest distribution you intend to support.** A binary links
against the glibc it was built with and will not start on anything older, so:

- built on RHEL 9 → runs on RHEL 9 and 10, Ubuntu 25.04, Manjaro
- built on Ubuntu 25.04 → does **not** run on RHEL 9

This applies to the AppImage too: it carries its own libraries but still uses
the host's glibc. A container is the easy way to get the old one:

```sh
podman run --rm -v "$PWD:/src" -w /src rockylinux:9 sh -c '
    dnf install -y gcc cmake perl python3 openssl-devel &&
    curl --proto "=https" -sSf https://sh.rustup.rs | sh -s -- -y &&
    . "$HOME/.cargo/env" &&
    packaging/linux/portable.sh'
```

`--features bundled` — which every Linux script passes — builds libgit2 and
OpenSSL from source rather than linking whatever the distribution ships, so the
binary does not care which versions RHEL and Ubuntu happen to have. It makes
the build slower and the binary larger, and it is the whole reason one tarball
can cover three distributions.

## What cannot be put in the box

Two things no packaging can carry, both worth knowing before someone reports
them as bugs:

- **A graphics driver.** Rendering prefers Vulkan and falls back to software
  when there is none, so gitDruid starts on a machine with no GPU driver — just
  slowly. `mesa-vulkan-drivers` is a recommendation rather than a requirement
  for exactly this reason.
- **A font.** The whole window is monospaced, resolved through fontconfig. A
  desktop always has one; a minimal container often does not, and the app will
  render boxes. `dejavu-sans-mono-fonts` or equivalent fixes it.

## macOS signing

`bundle.sh` signs ad-hoc, which stops macOS calling the app damaged on Apple
Silicon but is not a Developer ID signature. Anyone who did not build it
themselves still has to right-click → Open the first time, or:

```sh
xattr -d com.apple.quarantine /Applications/gitDruid.app
```

To hand it out without that, with an Apple Developer account:

```sh
codesign --force --deep --options runtime \
    --sign "Developer ID Application: Your Name (TEAMID)" gitDruid.app

ditto -c -k --keepParent gitDruid.app gitDruid.zip
xcrun notarytool submit gitDruid.zip --apple-id you@example.com \
    --team-id TEAMID --password APP_SPECIFIC_PASSWORD --wait
xcrun stapler staple gitDruid.app
```

`--universal` builds for Apple Silicon and Intel and joins them with `lipo`; it
needs both `rustup` targets installed and says so if they are missing.

## What has actually been run

Being straight about this, because the scripts were written on a Mac:

- **macOS bundle: built and launched.** `gitDruid.app` assembles, passes
  `codesign --verify --strict`, and opens by double-click.
- **The rootless installer: run for real**, against a sandboxed `HOME`. It
  creates the binary, the launcher and seven icon sizes, rewrites `Exec` to an
  absolute path, warns when `~/.local/bin` is off `PATH`, and removes every
  file it made.
- **The icon: generated and inspected** at every size down to 16px.
- **The Linux build scripts, the RPM spec, the deb and the PKGBUILD: not run.**
  They are syntax-checked and the layouts follow the conventions, but no Linux
  machine has executed them. Expect to fix something the first time, most
  likely a `BuildRequires` line that is missing a package one distribution
  splits differently.
