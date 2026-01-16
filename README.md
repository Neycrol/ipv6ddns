# ipv6ddns

Event-driven IPv6 DDNS client for Cloudflare, written in Rust.

## Features

- **Event-driven**: Uses netlink for real-time IPv6 change events (fallback to polling)
- **Correctness-first**: Filters out temporary/tentative/deprecated addresses
- **Lightweight**: ~1MB memory footprint, zero runtime dependencies
- **Secure**: Sensitive data via environment variables (no secrets in config)
- **Reliable**: Automatic retry with exponential backoff on failures

## Requirements

- Rust 1.70+
- Linux with netlink support
- Cloudflare API Token with DNS edit permissions

## Installation

### Linux

```bash
# Build
cargo build --release

# Install
sudo install -m 755 target/release/ipv6ddns /usr/local/bin/
sudo mkdir -p /etc/ipv6ddns
sudo cp etc/config.toml /etc/ipv6ddns/config.toml
sudo cp etc/ipv6ddns.service /etc/systemd/system/

# Set environment variables (choose one method below)
```

### Android

The ipv6ddns Android app provides a convenient UI for managing the DDNS service on your Android device.

**Features:**
- Native Android UI with Material Design 3
- Secure credential storage (API tokens are never exposed)
- Foreground service with persistent notification
- Real-time status monitoring
- Configurable timeout and polling intervals
- Multi-record policy selection
- Automatic binary download and verification

**Installation:**
1. Download the latest APK from the [Releases](https://github.com/Neycrol/ipv6ddns/releases) page
2. Enable "Install from unknown sources" in your device settings
3. Install the APK
4. Grant necessary permissions (foreground service, network access)

**Setup:**
1. Open the ipv6ddns app
2. Enter your Cloudflare API Token
3. Enter your Zone ID
4. Enter the DNS record name to update (e.g., `home.example.com`)
5. Configure optional settings (timeout, polling interval, verbose logging)
6. Tap "Start" to begin monitoring

**Troubleshooting Android:**
- **Service stops unexpectedly:** Check if battery optimization is affecting the app. Add ipv6ddns to the battery whitelist.
- **No IPv6 detected:** Ensure your device has a global IPv6 address. Check in Settings > Network.
- **Binary verification fails:** Clear app data and try reinstalling. Ensure you have a stable internet connection.
- **Logs not visible:** Enable verbose logging in the app settings and check logcat: `adb logcat | grep ipv6ddns`

**Building from source:**
```bash
# Install Android SDK and NDK
export ANDROID_NDK_HOME=/path/to/ndk

# Build Android assets
./scripts/ci/build-android.sh

# Build APK
cd android
gradle assembleRelease
```

## Configuration

### Method 1: Environment Variables (Recommended)

Set variables in systemd service file:

```bash
sudo systemctl edit ipv6ddns
```

Add:
```ini
[Service]
Environment="CLOUDFLARE_API_TOKEN=your-token-here"
Environment="CLOUDFLARE_ZONE_ID=your-zone-id"
Environment="CLOUDFLARE_RECORD_NAME=home.example.com"
Environment="CLOUDFLARE_MULTI_RECORD=error"
```

Or create `/etc/default/ipv6ddns`:
```bash
export CLOUDFLARE_API_TOKEN="your-token-here"
export CLOUDFLARE_ZONE_ID="your-zone-id"
export CLOUDFLARE_RECORD_NAME="home.example.com"
export CLOUDFLARE_MULTI_RECORD="error"
```

### Method 2: Config File

Edit `/etc/ipv6ddns/config.toml`:

```toml
record_name = "home.example.com"
timeout = 30
# Optional, but env vars override these when set:
# api_token = "your-token-here"
# zone_id = "your-zone-id"
verbose = false
multi_record = "error" # error|first|all
# Sensitive values via environment variables (recommended)
```

`multi_record` controls behavior when multiple AAAA records exist for the same name:
- `error` (default): refuse to update
- `first`: update the first record found
- `all`: update all matching AAAA records

### Getting Cloudflare credentials

1. Go to [Cloudflare Dashboard](https://dash.cloudflare.com/profile/api-tokens)
2. Create a token with `Zone:DNS:Edit` permissions
3. Get Zone ID from your domain's DNS settings page

### Enable and start

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ipv6ddns
```

## Architecture

```
┌─────────────────────────────────────┐
│           ipv6ddns Daemon           │
├─────────────────────────────────────┤
│  IPv6 Netlink Monitor               │
│  (fallback: polling interval)       │
│          ↓                          │
│  State Machine                      │
│  - Unknown → Detected → Synced      │
│  - Error (with backoff)             │
│          ↓                          │
│  Cloudflare API Client              │
│  - GET existing record              │
│  - PUT update / POST create         │
└─────────────────────────────────────┘
```

## Logs

```bash
journalctl -u ipv6ddns -f
```

## License

MIT
