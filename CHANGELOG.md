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
- Configuration hot-reload capability via SIGHUP signal
- `--config-test` command-line parameter for config validation

### Changed
- Enhanced CI/CD pipeline with test coverage reporting
- Improved CHANGELOG validation in release workflow
- Added rustdoc checks to CI pipeline

### Fixed
- Android test timeout handling in CI

## [1.0.0] - 2026-01-19

### Added
- Event-driven IPv6 DDNS client for Cloudflare
- Netlink-based IPv6 address monitoring
- Automatic DNS record updates with exponential backoff
- Android companion app
