#!/bin/bash
#==============================================================================
# ipv6ddns Build & Deploy Script
#
# Builds the Rust application and deploys it to the system.
#==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/target/release/ipv6ddns"
CONFIG_SRC="${SCRIPT_DIR}/etc/config.toml"
CONFIG_DST="/etc/ipv6ddns/config.toml"
SERVICE_SRC="${SCRIPT_DIR}/etc/ipv6ddns.service"
SERVICE_DST="/etc/systemd/system/ipv6ddns.service"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

#==============================================================================
# Build
#==============================================================================

build() {
    log_info "Building ipv6ddns..."

    cd "${SCRIPT_DIR}"

    # Check for Rust toolchain
    if ! command -v cargo &> /dev/null; then
        log_error "Rust toolchain not found. Install from https://rustup.rs/"
        exit 1
    fi

    # Build with release optimizations
    cargo build --release

    if [[ ! -f "${BINARY}" ]]; then
        log_error "Build failed: binary not found"
        exit 1
    fi

    # Strip binary for smaller size
    strip --strip-all "${BINARY}"

    log_info "Build successful: ${BINARY}"
    log_info "Binary size: $(du -h "${BINARY}" | cut -f1)"
}

#==============================================================================
# Install
#==============================================================================

install() {
    log_info "Installing ipv6ddns..."

    cd "${SCRIPT_DIR}"

    # Create config directory
    sudo mkdir -p /etc/ipv6ddns

    # Copy binary
    if [[ ! -f "${BINARY}" ]]; then
        log_error "Binary not found. Run 'build' first."
        exit 1
    fi
    sudo install -m 755 "${BINARY}" /usr/local/bin/ipv6ddns

    # Copy config (preserve existing if it exists)
    if [[ -f "${CONFIG_DST}" ]]; then
        log_warn "Config already exists at ${CONFIG_DST}, skipping..."
    else
        sudo install -m 600 "${CONFIG_SRC}" "${CONFIG_DST}"
        log_warn "Config installed, please edit ${CONFIG_DST} with your API credentials"
    fi

    # Install systemd service
    sudo install -m 644 "${SERVICE_SRC}" "${SERVICE_DST}"

    # Reload systemd and enable service
    sudo systemctl daemon-reload
    sudo systemctl enable ipv6ddns.service

    log_info "Installation complete"
}

#==============================================================================
# Uninstall
#==============================================================================

uninstall() {
    log_info "Uninstalling ipv6ddns..."

    # Stop and disable service
    sudo systemctl stop ipv6ddns.service 2>/dev/null || true
    sudo systemctl disable ipv6ddns.service 2>/dev/null || true

    # Remove files
    sudo rm -f /usr/local/bin/ipv6ddns
    sudo rm -f /etc/systemd/system/ipv6ddns.service
    sudo rm -rf /etc/ipv6ddns

    sudo systemctl daemon-reload

    log_info "Uninstall complete"
}

#==============================================================================
# Status
#==============================================================================

status() {
    echo "=== ipv6ddns Status ==="
    systemctl status ipv6ddns.service --no-pager || true
    echo ""
    echo "=== Recent Logs ==="
    journalctl -u ipv6ddns --since "1 hour ago" --no-pager || true
}

#==============================================================================
# Logs
#==============================================================================

logs() {
    journalctl -u ipv6ddns --follow "${@}"
}

#==============================================================================
# Help
#==============================================================================

help() {
    cat << EOF
Usage: $(basename "$0") <command>

Commands:
    build      Build the application
    install    Install to system
    uninstall  Remove from system
    status     Show service status
    logs       Show service logs (tail -f)
    help       Show this help

Examples:
    $(basename "$0") build
    $(basename "$0") install
    $(basename "$0") status
EOF
}

#==============================================================================
# Main
#==============================================================================

main() {
    local command="${1:-help}"

    case "${command}" in
        build)      build ;;
        install)    install ;;
        uninstall)  uninstall ;;
        status)     status ;;
        logs)       logs "${@:2}" ;;
        help|--help|-h) help ;;
        *)
            log_error "Unknown command: ${command}"
            help
            exit 1
            ;;
    esac
}

main "${@}"
