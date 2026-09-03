# RPM for RHEL 9 and 10, and anything else using rpm.
#
# Build from a source tarball:
#   rpmbuild -ba packaging/linux/gitdruid.spec
#
# BuildRequires covers what iced and libgit2 need to compile. The runtime
# requirements are deliberately few: everything else is either statically
# linked or, in the case of the graphics driver, a thing no package can carry.

Name:           gitdruid
Version:        0.0.5
Release:        1%{?dist}
Summary:        A cross-platform git GUI focused on building good commits

License:        MIT
URL:            https://github.com/raparkhurst/gitDruid
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  cmake
BuildRequires:  perl
BuildRequires:  python3
BuildRequires:  pkgconfig(libxkbcommon)
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  desktop-file-utils

Requires:       fontconfig
Requires:       libxkbcommon
# Rendering falls back to software when there is no Vulkan driver, so mesa is
# recommended rather than required: the app starts either way.
Recommends:     mesa-vulkan-drivers

%description
gitDruid shows what changed, stages it a hunk at a time, and writes the
message. It draws the history of every ref as a coloured graph, manages
branches and tags under a configurable workflow, and fetches, pulls and
pushes. Several repositories can be open at once, one per tab.

%prep
%autosetup

%build
cargo build --release --features bundled

%install
install -D -m 0755 target/release/git-druid %{buildroot}%{_bindir}/git-druid

python3 packaging/icon.py packaging/icons

for size in 16 32 48 64 128 256 512; do
    install -D -m 0644 packaging/icons/${size}.png \
        %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/gitdruid.png
done

desktop-file-install --dir=%{buildroot}%{_datadir}/applications \
    packaging/gitdruid.desktop

%files
%license LICENSE
%doc README.md
%{_bindir}/git-druid
%{_datadir}/applications/gitdruid.desktop
%{_datadir}/icons/hicolor/*/apps/gitdruid.png

%changelog
* Wed Sep 03 2026 Robert Parkhurst <raparkhurst@digitalsynapse.io> - 0.0.5-1
- First packaged release.
