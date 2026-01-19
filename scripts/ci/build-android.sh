#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ASSETS_DIR="$ROOT/android/app/src/main/assets"

mkdir -p "$ASSETS_DIR"
rm -f "$ASSETS_DIR"/ipv6ddns-*

if ! command -v cargo-ndk >/dev/null 2>&1; then
  cargo install cargo-ndk --locked
fi

rustup target add aarch64-linux-android x86_64-linux-android

echo "Building rust binary for arm64-v8a..."
cargo ndk -t arm64-v8a build --release
cp "$ROOT/target/aarch64-linux-android/release/ipv6ddns" "$ASSETS_DIR/ipv6ddns-arm64-v8a"
sha256sum "$ASSETS_DIR/ipv6ddns-arm64-v8a" > "$ASSETS_DIR/ipv6ddns-arm64-v8a.sha256"

echo "Building rust binary for x86_64..."
cargo ndk -t x86_64 build --release
cp "$ROOT/target/x86_64-linux-android/release/ipv6ddns" "$ASSETS_DIR/ipv6ddns-x86_64"
sha256sum "$ASSETS_DIR/ipv6ddns-x86_64" > "$ASSETS_DIR/ipv6ddns-x86_64.sha256"
