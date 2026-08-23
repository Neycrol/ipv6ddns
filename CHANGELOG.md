# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- DNS provider abstraction layer for multi-provider support
- HTTP health check endpoint
- HTTP connection pool optimization for better performance
- Minimum Supported Rust Version (MSRV) specification

### Changed
- Migrated to Rust edition 2024 (MSRV now 1.85)
- Upgraded dependencies: tokio 1.53, reqwest 0.13 (new `rustls` + `webpki-roots` features), clap 4.6, toml 1.1, zeroize 1.9, serial_test 4.0, tempfile 3.27
- Replaced `async-trait` with native `async fn` in traits: `Daemon` is now generic over `DnsProvider` and the netlink monitor uses enum dispatch instead of `Box<dyn>` (static dispatch, no boxing)
- CPU-path optimizations:
  - Netlink events and sync state now carry native `Ipv6Addr` (16-byte `Copy`) instead of `String`; text conversion happens only at logging/API boundaries, eliminating per-event heap allocations and parse/format round-trips
  - The event-driven receive path reuses a preallocated buffer instead of allocating (and zeroing) 8 KiB per netlink message
  - `redact_secrets` returns a borrowed string when nothing needs redacting
  - Record-change detection compares parsed addresses (also normalizes formatting differences from the API)
  - Release profile uses `panic = "abort"` (daemon is systemd-supervised; smaller binary)

### Removed
- Removed `async-trait` and `urlencoding` dependencies (record names and zone IDs are validated to a URL-safe charset before being used in API paths/queries)

### Changed
- Enhanced CI/CD pipeline with test coverage reporting
- Improved CHANGELOG validation in release workflow
- Added rustdoc checks to CI pipeline

### Fixed
- **Busy-spin after the first netlink event**: the netlink receive helper swallowed `WouldBlock` instead of propagating it, preventing tokio's `AsyncFd` from clearing socket readiness. `readable().await` then resolved instantly forever, pinning a full CPU core on the single-threaded runtime and starving every other task (including the health endpoint). Found by profiling the real binary under load — CPU sampling plus strace — not by review alone.
- Android test timeout handling in CI

## [1.0.0] - 2026-01-19

### Added
- Event-driven IPv6 DDNS client for Cloudflare
- Netlink-based IPv6 address monitoring
- Automatic DNS record updates with exponential backoff
- Android companion app
