#!/bin/bash
# Dependency Audit Script
# This script performs comprehensive security auditing of project dependencies
# It checks for known vulnerabilities, outdated dependencies, and license compliance

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Exit codes
EXIT_SUCCESS=0
EXIT_VULNERABILITIES_FOUND=1
EXIT_OUTDATED_DEPENDENCIES=2
EXIT_LICENSE_ISSUES=3
EXIT_TOOLS_MISSING=4

# Check if required tools are installed
check_requirements() {
    log_info "Checking requirements..."
    
    local missing_tools=()
    
    if ! command -v cargo &> /dev/null; then
        missing_tools+=("cargo")
    fi
    
    if ! command -v cargo-audit &> /dev/null; then
        missing_tools+=("cargo-audit")
    fi
    
    if ! command -v cargo-deny &> /dev/null; then
        missing_tools+=("cargo-deny")
    fi
    
    if ! command -v cargo-outdated &> /dev/null; then
        missing_tools+=("cargo-outdated")
    fi
    
    if [ ${#missing_tools[@]} -ne 0 ]; then
        log_error "Missing required tools: ${missing_tools[*]}"
        log_info "Install them with:"
        log_info "  cargo install cargo-audit cargo-deny cargo-outdated"
        exit $EXIT_TOOLS_MISSING
    fi
    
    log_info "All requirements satisfied"
}

# Run cargo audit to check for known vulnerabilities
run_cargo_audit() {
    log_info "Running cargo audit..."
    
    if ! cargo audit; then
        log_error "Vulnerabilities found by cargo audit"
        return $EXIT_VULNERABILITIES_FOUND
    fi
    
    log_info "No known vulnerabilities found"
    return $EXIT_SUCCESS
}

# Run cargo deny to check licenses and bans
run_cargo_deny() {
    log_info "Running cargo deny..."
    
    local deny_exit_code=0
    
    # Check licenses
    if ! cargo deny check licenses; then
        log_error "License check failed"
        deny_exit_code=$EXIT_LICENSE_ISSUES
    fi
    
    # Check bans
    if ! cargo deny check bans; then
        log_error "Banned dependencies found"
        deny_exit_code=$EXIT_LICENSE_ISSUES
    fi
    
    # Check sources
    if ! cargo deny check sources; then
        log_error "Source check failed"
        deny_exit_code=$EXIT_LICENSE_ISSUES
    fi
    
    if [ $deny_exit_code -eq 0 ]; then
        log_info "All cargo deny checks passed"
    fi
    
    return $deny_exit_code
}

# Check for outdated dependencies
run_cargo_outdated() {
    log_info "Checking for outdated dependencies..."
    
    # Run cargo outdated and capture output
    local outdated_output
    outdated_output=$(cargo outdated --format json 2>&1 || true)
    
    # Check if there are outdated dependencies
    if echo "$outdated_output" | grep -q '"dependencies"'; then
        log_warn "Outdated dependencies found:"
        cargo outdated --format list
        return $EXIT_OUTDATED_DEPENDENCIES
    fi
    
    log_info "All dependencies are up to date"
    return $EXIT_SUCCESS
}

# Check for duplicate dependencies
run_duplicate_check() {
    log_info "Checking for duplicate dependencies..."
    
    # Use cargo tree to find duplicates
    local duplicates
    duplicates=$(cargo tree --duplicates | grep -c "^duplicate" || true)
    
    if [ "$duplicates" -gt 0 ]; then
        log_warn "Found $duplicates duplicate dependencies:"
        cargo tree --duplicates
        return $EXIT_OUTDATED_DEPENDENCIES
    fi
    
    log_info "No duplicate dependencies found"
    return $EXIT_SUCCESS
}

# Generate a comprehensive report
generate_report() {
    log_info "Generating audit report..."
    
    local report_file="dependency-audit-report.md"
    
    cat > "$report_file" << EOF
# Dependency Audit Report

Generated on: $(date)

## Summary

This report contains the results of the dependency audit for the ipv6ddns project.

## Vulnerability Check

$(cargo audit 2>&1 || echo "cargo audit failed")

## License Check

$(cargo deny check licenses 2>&1 || echo "cargo deny licenses check failed")

## Outdated Dependencies

$(cargo outdated --format list 2>&1 || echo "cargo outdated failed")

## Duplicate Dependencies

$(cargo tree --duplicates 2>&1 || echo "No duplicates found")

## Recommendations

1. Review any vulnerabilities found by cargo audit
2. Update outdated dependencies when possible
3. Resolve any license compatibility issues
4. Minimize duplicate dependencies to reduce binary size

EOF
    
    log_info "Report generated: $report_file"
}

# Main audit function
run_audit() {
    local exit_code=$EXIT_SUCCESS
    
    log_info "Starting dependency audit..."
    
    # Run all checks
    if ! run_cargo_audit; then
        exit_code=$EXIT_VULNERABILITIES_FOUND
    fi
    
    if ! run_cargo_deny; then
        exit_code=$EXIT_LICENSE_ISSUES
    fi
    
    if ! run_cargo_outdated; then
        exit_code=$EXIT_OUTDATED_DEPENDENCIES
    fi
    
    if ! run_duplicate_check; then
        exit_code=$EXIT_OUTDATED_DEPENDENCIES
    fi
    
    # Generate report
    generate_report
    
    # Summary
    echo
    log_info "Audit completed"
    
    case $exit_code in
        $EXIT_SUCCESS)
            log_info "✅ No issues found"
            ;;
        $EXIT_VULNERABILITIES_FOUND)
            log_error "❌ Vulnerabilities found"
            ;;
        $EXIT_OUTDATED_DEPENDENCIES)
            log_warn "⚠️  Outdated or duplicate dependencies found"
            ;;
        $EXIT_LICENSE_ISSUES)
            log_error "❌ License or policy violations found"
            ;;
        *)
            log_error "❌ Unknown error"
            ;;
    esac
    
    return $exit_code
}

# Help function
show_help() {
    cat << EOF
Dependency Audit Script

This script performs comprehensive security auditing of project dependencies.

Usage: $0 [OPTIONS]

Options:
    -h, --help          Show this help message
    -r, --report        Generate audit report only (don't run checks)
    -v, --verbose       Enable verbose output
    -q, --quiet         Suppress informational messages

Exit Codes:
    $EXIT_SUCCESS              - No issues found
    $EXIT_VULNERABILITIES_FOUND - Vulnerabilities detected
    $EXIT_OUTDATED_DEPENDENCIES - Outdated or duplicate dependencies
    $EXIT_LICENSE_ISSUES       - License/policy violations
    $EXIT_TOOLS_MISSING        - Required tools not installed

Examples:
    $0                      # Run full audit
    $0 --report             # Generate report only
    $0 --verbose            # Run with verbose output

EOF
}

# Parse command line arguments
VERBOSE=false
GENERATE_ONLY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_help
            exit $EXIT_SUCCESS
            ;;
        -r|--report)
            GENERATE_ONLY=true
            shift
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        -q|--quiet)
            # Redirect output to /dev/null except for errors
            exec > /dev/null
            shift
            ;;
        *)
            log_error "Unknown option: $1"
            show_help
            exit $EXIT_TOOLS_MISSING
            ;;
    esac
done

# Main execution
main() {
    # Check requirements
    check_requirements
    
    if [ "$GENERATE_ONLY" = true ]; then
        generate_report
        exit $EXIT_SUCCESS
    fi
    
    # Run audit
    run_audit
}

# Run main function
main "$@"
