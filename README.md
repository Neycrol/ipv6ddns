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

## License

MIT

<!-- gh pr test 2026-01-15 -->
