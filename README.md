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

## Troubleshooting

### Netlink socket fails

If you see "Netlink socket failed" in the logs, the daemon will automatically fall back to polling mode. This can happen if:
- The system doesn't support netlink (unlikely on Linux)
- The process lacks sufficient permissions

**Solution:** Ensure the daemon runs with appropriate permissions. Polling mode works but uses more CPU.

### Rate limiting from Cloudflare

If you see "Rate limited by Cloudflare" errors:
- The daemon uses exponential backoff with a maximum of 10 minutes
- Reduce the frequency of IPv6 address changes if possible
- Check if other processes are hitting the Cloudflare API

**Solution:** Wait for the backoff period to expire. The daemon will automatically retry.

### Multiple AAAA records exist

If you see "Multiple AAAA records found" errors:
- By default, the daemon refuses to update when multiple records exist for safety
- This prevents accidental updates to unintended records

**Solution:** Set `multi_record` to `first` or `all` in the config:
```toml
multi_record = "first"  # Update only the first record
# or
multi_record = "all"    # Update all matching records
```

### No IPv6 address detected

If the daemon reports "No IPv6 on startup":
- Check that your system has a global IPv6 address: `ip -6 addr show`
- Ensure the address is not temporary, tentative, or deprecated
- Verify netlink is working or polling is configured

**Solution:**
```bash
# Check for global IPv6 addresses
ip -6 addr show scope global

# If using polling, check the poll_interval setting
```

### Permission denied errors

If you see permission errors:
- The daemon needs CAP_NET_RAW capability for netlink
- Systemd service should run with appropriate permissions

**Solution:** Ensure the systemd service is correctly installed:
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ipv6ddns
```

### API token errors

If you see API authentication errors:
- Verify your API token has `Zone:DNS:Edit` permissions
- Check that the zone ID is correct
- Ensure the token hasn't expired

**Solution:** Re-create the API token with correct permissions from the Cloudflare Dashboard.

### Service won't start

If the systemd service fails to start:
- Check the service status: `systemctl status ipv6ddns`
- View detailed logs: `journalctl -u ipv6ddns -n 50`
- Verify the config file exists and is valid

**Solution:**
```bash
# Check service status
sudo systemctl status ipv6ddns

# View recent logs
sudo journalctl -u ipv6ddns -n 50

# Test config syntax
cat /etc/ipv6ddns/config.toml
```

### DNS record not updating

If the daemon runs but DNS doesn't update:
- Check logs for API errors
- Verify the record name matches exactly (including subdomains)
- Ensure the zone ID is correct for your domain

**Solution:** Enable verbose logging temporarily:
```toml
verbose = true
```

## License

MIT
