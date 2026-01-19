#!/usr/bin/env bash
set -euo pipefail

ARTIFACTS_DIR="${1:?artifacts directory required}"

echo "Verifying release artifacts in $ARTIFACTS_DIR"

# Check if directory exists
if [ ! -d "$ARTIFACTS_DIR" ]; then
    echo "ERROR: Artifacts directory does not exist: $ARTIFACTS_DIR"
    exit 1
fi

# Check .deb files
find "$ARTIFACTS_DIR" -name "*.deb" -type f | while read -r deb; do
    echo "Checking $deb"
    dpkg-deb --info "$deb" > /dev/null
    tmp_contents="$(mktemp)"
    dpkg-deb --contents "$deb" > "$tmp_contents"
    grep -q "usr/bin/ipv6ddns" "$tmp_contents"
    rm -f "$tmp_contents"
    echo "  ✓ Valid .deb package"
done

# Check AppImage files
find "$ARTIFACTS_DIR" -name "*.AppImage" -type f | while read -r appimage; do
    echo "Checking $appimage"
    chmod +x "$appimage"
    "$appimage" --appimage-extract >/dev/null
    test -f squashfs-root/usr/bin/ipv6ddns
    rm -rf squashfs-root
    echo "  ✓ Valid AppImage"
done

# Check binary files
find "$ARTIFACTS_DIR" -name "ipv6ddns-*" -type f -executable | while read -r bin; do
    echo "Checking $bin"
    file "$bin" | grep -q "ELF"
    echo "  ✓ Valid ELF binary"
done

# Check Arch package files
find "$ARTIFACTS_DIR" -name "*.pkg.tar.zst" -type f | while read -r pkg; do
    echo "Checking $pkg"
    tar --zstd -tf "$pkg" | grep -q "usr/bin/ipv6ddns"
    echo "  ✓ Valid Arch package"
done

# Check APK files
find "$ARTIFACTS_DIR" -name "*.apk" -type f | while read -r apk; do
    echo "Checking $apk"
    unzip -t "$apk" >/dev/null
    echo "  ✓ Valid APK package"
done

# Check SHA256 files
find "$ARTIFACTS_DIR" -name "*.sha256" -type f | while read -r sha; do
    echo "Checking $sha"
    # Verify SHA256 format
    grep -qE '^[a-f0-9]{64}  ' "$sha"
    echo "  ✓ Valid SHA256 file"
done

echo ""
echo "All artifacts verified successfully!"
