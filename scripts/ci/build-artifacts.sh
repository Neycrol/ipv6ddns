#!/usr/bin/env bash
set -euo pipefail

ARCH=${1:?arch required (x86_64|aarch64)}
APPIMAGE_TOOL_URL=${APPIMAGE_TOOL_URL:-auto}

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/../.." && pwd)
cd "$ROOT_DIR"

# Clean previous artifacts
rm -rf "$ROOT_DIR/dist"
VERSION=$(awk -F '\"' '/^version =/ {print $2; exit}' Cargo.toml)
GIT_SHA=$(git rev-parse --short HEAD)
PKGVER="${VERSION}+git.${GIT_SHA}"

export CARGO_PROFILE_RELEASE_LTO="thin"
export RUSTFLAGS="-C opt-level=3 -C codegen-units=1"

TARGET_TRIPLE=${CARGO_BUILD_TARGET:-}
BIN_PATH="target/release/ipv6ddns"
if [ "$ARCH" = "aarch64" ]; then
  TARGET_TRIPLE=${TARGET_TRIPLE:-aarch64-unknown-linux-gnu}
  export CC_aarch64_unknown_linux_gnu=${CC_aarch64_unknown_linux_gnu:-aarch64-linux-gnu-gcc}
  export AR_aarch64_unknown_linux_gnu=${AR_aarch64_unknown_linux_gnu:-aarch64-linux-gnu-ar}
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}
  cargo build --release --target "$TARGET_TRIPLE"
  BIN_PATH="target/${TARGET_TRIPLE}/release/ipv6ddns"
else
  cargo build --release
fi

mkdir -p "$ROOT_DIR/dist"

# Build .deb manually (avoid cargo-deb build deps)
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) DEB_ARCH="$ARCH" ;;
esac

DEBROOT="$ROOT_DIR/dist/debroot"
rm -rf "$DEBROOT"
mkdir -p "$DEBROOT/DEBIAN" \
         "$DEBROOT/usr/bin" \
         "$DEBROOT/lib/systemd/system" \
         "$DEBROOT/etc/ipv6ddns" \
         "$DEBROOT/usr/share/doc/ipv6ddns"

install -m 755 "$BIN_PATH" "$DEBROOT/usr/bin/ipv6ddns"
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

dpkg-deb --build "$DEBROOT" "$ROOT_DIR/dist/ipv6ddns-${PKGVER}-${ARCH}.deb"
rm -rf "$DEBROOT"

# AppImage
APPDIR="$ROOT_DIR/dist/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
cp -f "$BIN_PATH" "$APPDIR/usr/bin/ipv6ddns"
cp -f packaging/ipv6ddns.desktop "$APPDIR/ipv6ddns.desktop"
cp -f packaging/ipv6ddns.svg "$APPDIR/ipv6ddns.svg"
cat > "$APPDIR/AppRun" <<'APP'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/ipv6ddns" "$@"
APP
chmod +x "$APPDIR/AppRun"

if [ "$APPIMAGE_TOOL_URL" = "auto" ]; then
  case "$ARCH" in
    x86_64)
      APPIMAGE_TOOL_URL="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
      ;;
    aarch64)
      APPIMAGE_TOOL_URL="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-aarch64.AppImage"
      ;;
    *)
      echo "No AppImageKit appimagetool for arch: ${ARCH}" >&2
      exit 1
      ;;
  esac
fi

curl -fsSL "$APPIMAGE_TOOL_URL" -o /tmp/appimagetool.AppImage
chmod +x /tmp/appimagetool.AppImage

# 验证 SHA-256 校验和
case "$APPIMAGE_TOOL_URL" in
  *appimagetool-x86_64.AppImage)
    EXPECTED_SHA256="b90f4a8b18967545fda78a445b27680a1642f1ef9488ced28b65398f2be7add2"
    ;;
  *appimagetool-aarch64.AppImage)
    EXPECTED_SHA256="a48972e5ae91c944c5a7c80214e7e0a42dd6aa3ae979d8756203512a74ff574d"
    ;;
  *)
    echo "WARNING: Unknown AppImageKit URL, skipping SHA-256 verification"
    EXPECTED_SHA256=""
    ;;
esac

if [ -n "$EXPECTED_SHA256" ]; then
  ACTUAL_SHA256=$(sha256sum /tmp/appimagetool.AppImage | awk '{print $1}')
  if [ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]; then
    echo "ERROR: SHA-256 mismatch for appimagetool.AppImage"
    echo "Expected: $EXPECTED_SHA256"
    echo "Actual: $ACTUAL_SHA256"
    exit 1
  fi
  echo "SHA-256 verification passed for appimagetool.AppImage"
fi

APPIMG_TMP=$(mktemp -d)
if APPIMAGE_EXTRACT_AND_RUN=1 /tmp/appimagetool.AppImage "$APPDIR" "$ROOT_DIR/dist/ipv6ddns-${PKGVER}-${ARCH}.AppImage"; then
  :
else
  # FUSE may be missing on CI; extract and run the embedded tool instead
  (cd "$APPIMG_TMP" && APPIMAGE_EXTRACT_AND_RUN=1 /tmp/appimagetool.AppImage --appimage-extract >/dev/null)
  APPIMAGETOOL_BIN="$APPIMG_TMP/squashfs-root/usr/bin/appimagetool"
  if [ -x "$APPIMAGETOOL_BIN" ]; then
    "$APPIMAGETOOL_BIN" "$APPDIR" "$ROOT_DIR/dist/ipv6ddns-${PKGVER}-${ARCH}.AppImage"
  else
    echo "appimagetool not found after extraction" >&2
    exit 1
  fi
fi
rm -rf "$APPIMG_TMP"
rm -rf "$APPDIR"

# Tarball (fallback / extra)
cp -f "$BIN_PATH" "$ROOT_DIR/dist/ipv6ddns-${PKGVER}-${ARCH}"
