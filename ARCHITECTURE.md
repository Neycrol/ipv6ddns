# Architecture

This document describes the architecture and design of ipv6ddns.

## Overview

ipv6ddns is an event-driven IPv6 DDNS client for Cloudflare. It monitors IPv6 address changes on the system and automatically updates Cloudflare DNS records.

## Design Principles

1. **Event-driven**: Uses netlink for real-time IPv6 address change events, falling back to polling if netlink is unavailable
2. **Correctness-first**: Filters out temporary, tentative, deprecated, and DAD-failed addresses
3. **Lightweight**: Small memory footprint in typical use; minimal runtime dependencies
4. **Secure**: Sensitive data (API tokens, zone IDs) via environment variables
5. **Reliable**: Automatic retry with exponential backoff on failures

## Components

### 1. Netlink Monitor (`src/netlink.rs`)

The netlink monitor is responsible for detecting IPv6 address changes on the system.

#### Event-driven Mode

- Creates a NETLINK_ROUTE socket with SOCK_RAW
- Subscribes to RTMGRP_IPV6_ADDR multicast group
- Receives RTM_NEWADDR and RTM_DELADDR events
- Zero CPU usage when idle (no polling)

#### Polling Mode (Fallback)

- Periodically checks for global IPv6 addresses using netlink dump
- Configurable polling interval (default: 60 seconds)
- Used when netlink socket creation fails

#### Address Filtering

The monitor filters out addresses that are:
- **Temporary**: Privacy extensions (IFA_F_TEMPORARY)
- **Tentative**: Address still being verified (IFA_F_TENTATIVE)
- **Deprecated**: No longer preferred (IFA_F_DEPRECATED)
- **DAD-failed**: Duplicate address detection failed (IFA_F_DADFAILED)
- **Non-global**: Not in RT_SCOPE_UNIVERSE scope

Only stable, global IPv6 addresses are processed.

### 2. Cloudflare Client (`src/cloudflare.rs`)

The Cloudflare client handles all interactions with the Cloudflare API.

#### API Operations

- **GET**: Retrieve existing AAAA records for a given name
- **POST**: Create a new AAAA record
- **PUT**: Update an existing AAAA record

#### Multi-record Policy

When multiple AAAA records exist for the same name, the client supports three policies:

- **Error** (default): Refuse to update, safest option
- **UpdateFirst**: Update only the first record found
- **UpdateAll**: Update all matching AAAA records

#### Error Handling

- Rate limiting (429): Triggers exponential backoff
- Server errors (5xx): Triggers exponential backoff
- Client errors (4xx): Returns error immediately

### 3. State Machine (`src/main.rs`)

The daemon uses a simple state machine to track record synchronization state.

#### States

- **Unknown**: Initial state, no record synced yet
- **Synced**: Record successfully synced with Cloudflare
- **Error**: Last sync attempt failed, in backoff period

#### State Transitions

```
Unknown → Synced: First successful sync
Synced → Synced: Successful sync with new IP
Synced → Error: Sync attempt failed
Error → Synced: Retry successful
Error → Error: Retry failed (increment error count)
```

#### Exponential Backoff

On errors, the daemon uses exponential backoff with these parameters:

- Base delay: 5 seconds
- Maximum delay: 10 minutes
- Exponent: 2^n (capped at 10)

Backoff formula: `min(5 * 2^(error_count - 1), 600)` seconds

### 4. Configuration (`src/main.rs`)

Configuration is loaded from multiple sources in order of precedence:

1. **Environment variables** (highest priority)
2. **Config file** (`/etc/ipv6ddns/config.toml`)
3. **Defaults** (lowest priority)

#### Required Fields

- `CLOUDFLARE_API_TOKEN`: Cloudflare API token with DNS edit permissions
- `CLOUDFLARE_ZONE_ID`: Cloudflare zone ID
- `CLOUDFLARE_RECORD_NAME`: DNS record name to update

#### Optional Fields

- `CLOUDFLARE_MULTI_RECORD`: Policy for multiple records (error|first|all)
- `timeout`: HTTP request timeout in seconds (default: 30)
- `poll_interval`: Polling interval in seconds (default: 60)
- `verbose`: Enable verbose logging (default: false)

### 5. Signal Handling

The daemon responds to Unix signals:

- **SIGTERM**: Graceful shutdown
- **SIGHUP**: Force resync (useful for manual trigger)

## Data Flow

### Normal Operation

```
┌─────────────────────────────────────────────────────────────────┐
│ 1. Netlink Monitor detects IPv6 address change                  │
│    - RTM_NEWADDR event or polling detects new IP                │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 2. State Machine checks current state                           │
│    - If already synced with same IP: skip                       │
│    - If in backoff period: skip                                 │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 3. Cloudflare Client queries existing records                   │
│    - GET /zones/{zone_id}/dns_records?name={name}&type=AAAA    │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 4. Multi-record Policy Check                                    │
│    - If multiple records and policy=Error: fail                 │
│    - Otherwise: proceed with update/create                      │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 5. Cloudflare Client updates or creates record                  │
│    - If record exists: PUT to update                            │
│    - If no record: POST to create                               │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 6. State Machine updates state                                  │
│    - On success: mark as Synced, reset error count              │
│    - On failure: mark as Error, start backoff                   │
└─────────────────────────────────────────────────────────────────┘
```

### Error Recovery

```
┌─────────────────────────────────────────────────────────────────┐
│ 1. Cloudflare API request fails                                 │
│    - Rate limited (429)                                         │
│    - Server error (5xx)                                         │
│    - Network error                                              │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 2. State Machine marks Error state                              │
│    - Increment error count                                      │
│    - Calculate backoff delay                                    │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 3. Subsequent IPv6 change events are ignored                    │
│    - Until backoff period expires                               │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ 4. Backoff period expires                                       │
│    - Next IPv6 change event triggers retry                      │
│    - If successful: mark as Synced                              │
│    - If failed: increment error count, restart backoff          │
└─────────────────────────────────────────────────────────────────┘
```

## Security Considerations

### Sensitive Data

- API tokens and zone IDs are never logged (redacted with `***REDACTED***`)
- Sensitive data should be provided via environment variables
- Config file can contain sensitive data, but environment variables override

### Input Validation

- DNS record names are validated for format and length
- IPv6 addresses are validated before API calls
- Multi-record policy values are validated

### Rate Limiting

- Respects Cloudflare rate limits (429 responses)
- Exponential backoff prevents hammering the API
- Maximum backoff of 10 minutes prevents excessive retries

### Netlink Security

- Uses SOCK_RAW with NETLINK_ROUTE (requires CAP_NET_RAW)
- Falls back to polling if netlink unavailable
- No privilege escalation attempts

## Performance Characteristics

### Memory Usage

- Small footprint in typical use (varies with runtime and build flags)
- Minimal heap allocations in steady state
- No data caching (stateless design)

### CPU Usage

- **Event-driven mode**: Near-zero CPU when idle (no polling)
- **Polling mode**: Periodic CPU usage every poll_interval
- **API requests**: CPU usage proportional to request rate

### Network Usage

- One HTTP request per IPv6 address change
- Minimal request/response size (JSON)
- No background traffic (event-driven)

## Testing

### Unit Tests

- Configuration loading and validation
- DNS record name validation
- Multi-record policy parsing
- API response parsing
- Netlink message parsing

### Integration Tests

- Full daemon lifecycle (not yet implemented)
- Mock Cloudflare API responses (not yet implemented)
- State machine transitions (not yet implemented)

## Future Enhancements

### Potential Improvements

1. **Configuration Reload**: Support reloading config without restart
2. **Multiple Records**: Support monitoring multiple DNS records
3. **Webhook Support**: Send webhook notifications on sync events
4. **IPv4 Support**: Add support for IPv4 DDNS

### Known Limitations

1. **Single Record**: Only monitors one DNS record per instance
2. **IPv6 Only**: No IPv4 support
3. **Linux Only**: Requires netlink (Linux-specific)
4. **No UI**: No graphical interface (except Android app)

## Observability

### Metrics

ipv6ddns exposes Prometheus metrics for monitoring:

- `ipv6ddns_dns_updates_total`: Total number of successful DNS updates
- `ipv6ddns_dns_errors_total`: Total number of DNS update errors
- `ipv6ddns_error_count`: Current number of consecutive errors
- `ipv6ddns_last_sync_seconds`: Time since last successful sync
- `ipv6ddns_sync_state`: Current sync state (0=Unknown, 1=Synced, 2=Error)
- `ipv6ddns_dns_update_duration_seconds`: DNS update duration histogram
- `ipv6ddns_ipv6_change_events`: IPv6 address change events histogram

Enable metrics by setting `IPV6DDNS_METRICS_PORT` environment variable (e.g., `9090`).

### Health Check

ipv6ddns provides a health check endpoint:

- `/health`: Returns health status JSON
- `/metrics`: Returns Prometheus metrics

Enable health check by setting `IPV6DDNS_HEALTH_PORT` environment variable (e.g., `8080`).

Example health response:
```json
{
  "status": "ok",
  "sync_state": "synced",
  "last_sync_seconds_ago": 0,
  "error_count": 0,
  "healthy": true
}
```

## DNS Provider Abstraction

ipv6ddns uses a trait-based abstraction for DNS providers, allowing support for multiple providers:

### Supported Providers

- **Cloudflare**: Default provider, fully supported

### Provider Configuration

Set the provider type via environment variable or config file:

```toml
provider_type = "cloudflare"
```

Or via environment variable:
```bash
export IPV6DDNS_PROVIDER_TYPE="cloudflare"
```

### Adding New Providers

To add support for a new DNS provider:

1. Implement the `DnsProvider` trait in a new module (e.g., `src/route53.rs`)
2. Add the provider to the configuration parsing logic
3. Update `src/main.rs` to instantiate the correct provider based on configuration

The `DnsProvider` trait requires implementing:

- `upsert_aaaa_record()`: Create or update an AAAA record
- `get_records()`: Retrieve existing AAAA records

## References

- [Netlink Protocol](https://man7.org/linux/man-pages/man7/netlink.7.html)
- [RTNETLINK](https://man7.org/linux/man-pages/man7/rtnetlink.7.html)
- [Cloudflare API](https://developers.cloudflare.com/api/)
- [IPv6 Address Types](https://en.wikipedia.org/wiki/IPv6_address#Address_scope)
