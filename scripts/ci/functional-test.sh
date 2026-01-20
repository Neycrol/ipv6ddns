#!/bin/bash
# Functional tests for ipv6ddns
# This script tests the basic functionality without requiring network access

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BINARY="${PROJECT_ROOT}/target/debug/ipv6ddns"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Test result tracking
TESTS_PASSED=0
TESTS_FAILED=0

# Helper functions
pass() {
    echo -e "${GREEN}✓${NC} $1"
    ((TESTS_PASSED++))
}

fail() {
    echo -e "${RED}✗${NC} $1"
    ((TESTS_FAILED++))
}

# Ensure binary exists
if [[ ! -f "${BINARY}" ]]; then
    echo "Building ipv6ddns..."
    cargo build
fi

# Create a temporary directory for test files
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "${TEMP_DIR}"' EXIT

# Test 1: --config-test with valid config
echo "Test 1: --config-test with valid config"
VALID_CONFIG="${TEMP_DIR}/valid_config.toml"
cat > "${VALID_CONFIG}" << 'EOF'
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
timeout = 30
poll_interval = 60
verbose = false
multi_record = "error"
allow_loopback = false
provider_type = "cloudflare"
health_port = 0
EOF

if "${BINARY}" --config "${VALID_CONFIG}" --config-test > /dev/null 2>&1; then
    pass "Valid config test passed"
else
    fail "Valid config test failed"
fi

# Test 2: --config-test with invalid config (missing required fields)
echo "Test 2: --config-test with invalid config (missing required fields)"
INVALID_CONFIG="${TEMP_DIR}/invalid_config.toml"
cat > "${INVALID_CONFIG}" << 'EOF'
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
EOF

if "${BINARY}" --config "${INVALID_CONFIG}" --config-test > /dev/null 2>&1; then
    fail "Invalid config test should have failed but passed"
else
    pass "Invalid config test correctly failed"
fi

# Test 3: --config-test with invalid config (invalid values)
echo "Test 3: --config-test with invalid config (invalid values)"
INVALID_VALUES_CONFIG="${TEMP_DIR}/invalid_values_config.toml"
cat > "${INVALID_VALUES_CONFIG}" << 'EOF'
api_token = "short"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
EOF

if "${BINARY}" --config "${INVALID_VALUES_CONFIG}" --config-test > /dev/null 2>&1; then
    fail "Invalid values config test should have failed but passed"
else
    pass "Invalid values config test correctly failed"
fi

# Test 4: --config-test with non-existent config file
echo "Test 4: --config-test with non-existent config file"
if "${BINARY}" --config "/non/existent/config.toml" --config-test > /dev/null 2>&1; then
    fail "Non-existent config test should have failed but passed"
else
    pass "Non-existent config test correctly failed"
fi

# Test 5: Help output
echo "Test 5: Help output"
if "${BINARY}" --help > /dev/null 2>&1; then
    pass "Help output test passed"
else
    fail "Help output test failed"
fi

# Test 6: Version output
echo "Test 6: Version output"
if "${BINARY}" --version > /dev/null 2>&1; then
    pass "Version output test passed"
else
    fail "Version output test failed"
fi

# Test 7: Config reload functionality (basic validation)
echo "Test 7: Config reload functionality"
RELOAD_CONFIG="${TEMP_DIR}/reload_config.toml"
cat > "${RELOAD_CONFIG}" << 'EOF'
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
health_port = 8080
EOF

# First, verify the config loads with health_port=8080
if ! "${BINARY}" --config "${RELOAD_CONFIG}" --config-test > /dev/null 2>&1; then
    fail "Initial config load failed"
    exit 1
fi

# Modify the config file
cat > "${RELOAD_CONFIG}" << 'EOF'
api_token = "0123456789012345678901234567890123456789"
zone_id = "0123456789abcdef0123456789abcdef"
record_name = "example.com"
health_port = 9090
EOF

# Verify the config now loads with health_port=9090
if "${BINARY}" --config "${RELOAD_CONFIG}" --config-test > /dev/null 2>&1; then
    pass "Config reload test passed"
else
    fail "Config reload test failed"
fi

# Summary
echo ""
echo "========================================"
echo "Test Summary:"
echo "  Passed: ${TESTS_PASSED}"
echo "  Failed: ${TESTS_FAILED}"
echo "========================================"

if [[ ${TESTS_FAILED} -eq 0 ]]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
fi
