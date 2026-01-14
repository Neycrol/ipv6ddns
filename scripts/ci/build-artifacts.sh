#!/usr/bin/env bash
set -euo pipefail

ARCH=${1:?arch required (x86_64|aarch64)}
APPIMAGE_TOOL_URL=${APPIMAGE_TOOL_URL:?APPIMAGE_TOOL_URL required}

ROOT_DIR=$(pwd)
VERSION=$(python3 - <<'PY'
import tomllib
with open('Cargo.toml','rb') as f:
    data=tomllib.load(f)
print(data['package']['version'])
PY
)
GIT_SHA=$(git rev-parse --short HEAD)
PKGVER="${VERSION}+git.${GIT_SHA}"

export CARGO_PROFILE_RELEASE_LTO="thin"
export RUSTFLAGS="-C opt-level=3 -C codegen-units=1 -C link-arg=-fuse-ld=lld"

cargo build --release

# Build .deb
cargo deb --no-build

mkdir -p dist

# Rename deb to include git sha + arch
DEB_SRC=$(ls -1 target/debian/*.deb | head -n1)
DEB_OUT="dist/ipv6ddns-${PKGVER}-${ARCH}.deb"
cp -f "$DEB_SRC" "$DEB_OUT"

# AppImage
rm -rf dist/AppDir
mkdir -p dist/AppDir/usr/bin
cp -f target/release/ipv6ddns dist/AppDir/usr/bin/ipv6ddns
cp -f packaging/ipv6ddns.desktop dist/AppDir/ipv6ddns.desktop
cp -f packaging/ipv6ddns.svg dist/AppDir/ipv6ddns.svg
cat > dist/AppDir/AppRun <<'APP'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/ipv6ddns" "$@"
APP
chmod +x dist/AppDir/AppRun

curl -fsSL "$APPIMAGE_TOOL_URL" -o /tmp/appimagetool.AppImage
chmod +x /tmp/appimagetool.AppImage

APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" /tmp/appimagetool.AppImage dist/AppDir "dist/ipv6ddns-${PKGVER}-${ARCH}.AppImage"

# Tarball (fallback / extra)
cp -f target/release/ipv6ddns "dist/ipv6ddns-${PKGVER}-${ARCH}"
