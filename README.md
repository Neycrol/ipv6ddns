# ipv6ddns

Event-driven IPv6 DDNS client for Cloudflare, written in Rust.

## Features

- **Event-driven**: Uses netlink for real-time IPv6 change events (fallback to polling)
- **Correctness-first**: Filters out temporary/tentative/deprecated addresses
- **Lightweight**: ~1MB memory footprint, zero runtime dependencies
- **Secure**: Sensitive data via environment variables (no secrets in config)
- **Reliable**: Automatic retry with exponential backoff on failures

## Requirements

- Rust 1.76+
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
Environment="IPV6DDNS_ALLOW_LOOPBACK=false"
```

Or create `/etc/default/ipv6ddns`:
```bash
export CLOUDFLARE_API_TOKEN="your-token-here"
export CLOUDFLARE_ZONE_ID="your-zone-id"
export CLOUDFLARE_RECORD_NAME="home.example.com"
export CLOUDFLARE_MULTI_RECORD="error"
export IPV6DDNS_ALLOW_LOOPBACK="false"
```

### Method 2: Config File

Edit `/etc/ipv6ddns/config.toml`:

```toml
record_name = "home.example.com"
timeout = 30 # 1-300 seconds
# Optional, but env vars override these when set:
# api_token = "your-token-here"
# zone_id = "your-zone-id"
verbose = false
multi_record = "error" # error|first|all
# allow_loopback = false # allow ::1 for local testing
# poll_interval = 60 # 10-3600 seconds (polling fallback)
# provider_type = "cloudflare" # DNS provider (default: cloudflare)
# health_port = 8080 # Health check port (0 = disabled)
# Sensitive values via environment variables (recommended)
```

### Health Check

ipv6ddns can expose a lightweight health check endpoint (disabled by default):

```bash
# Enable health check endpoint
export IPV6DDNS_HEALTH_PORT=8080
```

Or in config file:
```toml
health_port = 8080
```

Access endpoint:
- `http://localhost:8080/health` - Health check status

**Note:** For security, these endpoints bind to localhost only. Use a reverse proxy or SSH tunnel to access them remotely.

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

For detailed architecture documentation, see [ARCHITECTURE.md](ARCHITECTURE.md).

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

## Common Troubleshooting Scenarios

### Scenario 1: IPv6 address not detected

**Symptoms:**
- Daemon reports "No IPv6 on startup"
- DNS record is never updated

**Possible causes:**
1. No global IPv6 address assigned to the system
2. IPv6 address is temporary, tentative, or deprecated
3. Netlink socket is not available (fallback to polling)

**Solutions:**
```bash
# Check for global IPv6 addresses
ip -6 addr show scope global

# If using polling, check the poll_interval setting
cat /etc/ipv6ddns/config.toml

# Verify netlink is working
journalctl -u ipv6ddns | grep -i netlink
```

### Scenario 2: Cloudflare API authentication fails

**Symptoms:**
- "API error: Authentication error" in logs
- DNS record is never updated

**Possible causes:**
1. Invalid API token
2. API token lacks DNS edit permissions
3. API token has expired

**Solutions:**
```bash
# Verify API token has correct permissions
# Go to: https://dash.cloudflare.com/profile/api-tokens
# Required permission: Zone - DNS - Edit

# Check if zone ID is correct
curl -X GET "https://api.cloudflare.com/client/v4/zones/YOUR_ZONE_ID/dns_records" \
  -H "Authorization: Bearer YOUR_API_TOKEN"
# Tip: avoid pasting real tokens into shell history; use env vars or a temp file.
```

### Scenario 3: Rate limiting from Cloudflare

**Symptoms:**
- "Rate limited by Cloudflare" errors in logs
- DNS updates are delayed

**Possible causes:**
1. Too many API requests in a short time
2. Multiple instances running simultaneously

**Solutions:**
- The daemon uses exponential backoff (max 10 minutes)
- Wait for the backoff period to expire
- Reduce IPv6 address change frequency if possible

### Scenario 4: Multiple AAAA records exist

**Symptoms:**
- "Multiple AAAA records found" error in logs
- DNS record is not updated

**Possible causes:**
1. Multiple AAAA records with the same name exist in Cloudflare
2. Default policy refuses to update for safety

**Solutions:**
```toml
# Update /etc/ipv6ddns/config.toml
multi_record = "first"  # Update only the first record
# or
multi_record = "all"    # Update all matching records
```

### Scenario 5: Service won't start

**Symptoms:**
- Systemd service fails to start
- Service exits immediately

**Possible causes:**
1. Config file is missing or invalid
2. Required environment variables are not set
3. Binary does not have execute permissions

**Solutions:**
```bash
# Check service status
sudo systemctl status ipv6ddns

# View recent logs
sudo journalctl -u ipv6ddns -n 50

# Test config syntax
cat /etc/ipv6ddns/config.toml

# Verify environment variables
sudo systemctl show ipv6ddns -p Environment
```

### Scenario 6: DNS record not updating

**Symptoms:**
- Daemon runs but DNS doesn't update
- "Sync failed" errors in logs

**Possible causes:**
1. Record name doesn't match exactly
2. Zone ID is incorrect
3. Network connectivity issues

**Solutions:**
```bash
# Enable verbose logging temporarily
# Edit /etc/ipv6ddns/config.toml
verbose = true

# Restart service
sudo systemctl restart ipv6ddns

# Check logs for detailed error messages
sudo journalctl -u ipv6ddns -f
```

### Scenario 7: Android app issues

**Symptoms:**
- Service stops unexpectedly
- Binary verification fails
- No IPv6 detected

**Solutions:**
```bash
# Check logs
adb logcat | grep ipv6ddns

# Verify binary extraction
adb shell ls -la /data/data/com.neycrol.ipv6ddns/files/bin/

# Clear app data and reinstall
adb shell pm clear com.neycrol.ipv6ddns
```

### Scenario 8: Android binary extraction issues

**Symptoms:**
- "Security check failed: Binary checksum mismatch" error
- "Security check failed: Checksum file missing" error
- "Failed to extract binary" error
- Service fails to start

**Possible causes:**
1. Corrupted APK download
2. Incomplete app installation
3. Storage space issues
4. Device architecture incompatibility

**Solutions:**

**Checksum mismatch error:**
```bash
# This indicates the binary file on disk doesn't match the expected checksum
# Clear app data and reinstall:
adb shell pm clear com.neycrol.ipv6ddns
# Then reinstall the APK from a fresh download
```

**Checksum file missing error:**
```bash
# This indicates the .sha256 file is missing from the APK
# Reinstall the app from the official release page
# Verify the APK download completed successfully
```

**Failed to extract binary error:**
```bash
# Check available storage space
adb shell df -h /data/data/com.neycrol.ipv6ddns/

# If storage is low, free up space and retry
# Clear app data:
adb shell pm clear com.neycrol.ipv6ddns
```

**Architecture incompatibility:**
```bash
# Check device architecture
adb shell getprop ro.product.cpu.abi

# Supported architectures: arm64-v8a, x86_64
# If your device uses a different architecture, the app won't work
```

**General troubleshooting steps:**
1. Uninstall the app completely
2. Clear app data: `adb shell pm clear com.neycrol.ipv6ddns`
3. Download a fresh copy of the APK from the official releases page
4. Reinstall the APK
5. Grant necessary permissions (foreground service, network access)
6. Configure the app and start the service

**Binary verification details:**
The app uses SHA-256 checksums to verify the integrity of the extracted binary. The checksum file (e.g., `ipv6ddns-arm64-v8a.sha256`) must be present in the app assets and contain the correct checksum. If verification fails, the app will refuse to run the binary for security reasons.
```

## License

MIT
