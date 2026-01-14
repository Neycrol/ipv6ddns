#!/usr/bin/env bash
set -euo pipefail

ARCH=${1:?arch required (x86_64|aarch64)}
APPIMAGE_TOOL_URL=${APPIMAGE_TOOL_URL:?APPIMAGE_TOOL_URL required}

ROOT_DIR=$(pwd)
VERSION=$(awk -F '\"' '/^version =/ {print $2; exit}' Cargo.toml)
GIT_SHA=$(git rev-parse --short HEAD)
PKGVER="${VERSION}+git.${GIT_SHA}"

export CARGO_PROFILE_RELEASE_LTO="thin"
export RUSTFLAGS="-C opt-level=3 -C codegen-units=1 -C link-arg=-fuse-ld=lld"

cargo build --release

mkdir -p dist

# Build .deb manually (avoid cargo-deb build deps)
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) DEB_ARCH="$ARCH" ;;
esac

DEBROOT="dist/debroot"
rm -rf "$DEBROOT"
mkdir -p "$DEBROOT/DEBIAN" \
         "$DEBROOT/usr/bin" \
         "$DEBROOT/lib/systemd/system" \
         "$DEBROOT/etc/ipv6ddns" \
         "$DEBROOT/usr/share/doc/ipv6ddns"

install -m 755 target/release/ipv6ddns "$DEBROOT/usr/bin/ipv6ddns"
install -m 644 etc/ipv6ddns.service "$DEBROOT/lib/systemd/system/ipv6ddns.service"
install -m 644 etc/config.toml "$DEBROOT/etc/ipv6ddns/config.toml"
install -m 644 README.md "$DEBROOT/usr/share/doc/ipv6ddns/README.md"

cat > "$DEBROOT/DEBIAN/control" <<EOF
Package: ipv6ddns
Version: ${PKGVER}
Architecture: ${DEB_ARCH}
Maintainer: Neycrol <neycrol@users.noreply.github.com>
Section: net
Priority: optional
Depends: ca-certificates
Description: Event-driven IPv6 DDNS client for Cloudflare
EOF

dpkg-deb --build "$DEBROOT" "dist/ipv6ddns-${PKGVER}-${ARCH}.deb"

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
