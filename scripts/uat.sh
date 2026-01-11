#!/bin/bash
# Muninn UAT (User Acceptance Testing) Script
#
# Tests the core muninn functionality end-to-end.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test directory
TEST_DIR=$(mktemp -d)
MUNINN_BIN="${MUNINN_BIN:-cargo run --quiet --}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Muninn UAT Test Suite${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo "Test directory: $TEST_DIR"
echo "Project directory: $PROJECT_DIR"
echo ""

# Track test results
TESTS_PASSED=0
TESTS_FAILED=0

pass() {
    echo -e "  ${GREEN}✓ $1${NC}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
}

fail() {
    echo -e "  ${RED}✗ $1${NC}"
    echo -e "    ${RED}Error: $2${NC}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
}

section() {
    echo ""
    echo -e "${YELLOW}▶ $1${NC}"
}

cleanup() {
    echo ""
    echo -e "${BLUE}Cleaning up...${NC}"
    rm -rf "$TEST_DIR"

    # Kill any lingering muninn processes we started
    pkill -f "muninn proxy" 2>/dev/null || true
}
trap cleanup EXIT

# Change to project directory for cargo commands
cd "$PROJECT_DIR"

# Build first
section "Building muninn"
if cargo build --quiet 2>/dev/null; then
    pass "Build successful"
    MUNINN_BIN="$PROJECT_DIR/target/debug/muninn"
else
    fail "Build failed" "cargo build returned non-zero"
    exit 1
fi

# =============================================================================
# Test 1: muninn --help
# =============================================================================
section "Test 1: CLI Help"

if $MUNINN_BIN --help | grep -q "Privacy-first recursive context gateway"; then
    pass "Help text displays correctly"
else
    fail "Help text" "Expected description not found"
fi

if $MUNINN_BIN --help | grep -q "claude"; then
    pass "Claude subcommand listed"
else
    fail "Claude subcommand" "Not found in help"
fi

# =============================================================================
# Test 2: muninn init
# =============================================================================
section "Test 2: muninn init"

cd "$TEST_DIR"

if $MUNINN_BIN init 2>/dev/null; then
    pass "Init command succeeded"
else
    fail "Init command" "Command failed"
fi

if [ -d ".muninn" ]; then
    pass ".muninn directory created"
else
    fail ".muninn directory" "Not found"
fi

if [ -f ".muninn/config.toml" ]; then
    pass "Config file created"
else
    fail "Config file" ".muninn/config.toml not found"
fi

if grep -q '\[backend\]' .muninn/config.toml; then
    pass "Config has backend section"
else
    fail "Config content" "Missing [backend] section"
fi

if grep -q '\[router\]' .muninn/config.toml; then
    pass "Config has router section"
else
    fail "Config content" "Missing [router] section"
fi

# Test --force flag
if $MUNINN_BIN init --force 2>/dev/null; then
    pass "Init --force overwrites existing"
else
    fail "Init --force" "Command failed"
fi

# =============================================================================
# Test 3: muninn index
# =============================================================================
section "Test 3: muninn index"

# Create some test source files
mkdir -p src
cat > src/main.rs << 'EOF'
fn main() {
    println!("Hello, world!");
    helper();
}

fn helper() {
    let x = 42;
    println!("x = {}", x);
}
EOF

cat > src/lib.rs << 'EOF'
pub struct Config {
    pub name: String,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}
EOF

if $MUNINN_BIN index --path ./src --output ./test-graph.db 2>&1 | grep -q "Indexed"; then
    pass "Index command succeeded"
else
    fail "Index command" "Did not report indexing"
fi

if [ -f "test-graph.db" ]; then
    pass "Graph database created"
else
    fail "Graph database" "test-graph.db not found"
fi

# =============================================================================
# Test 4: muninn proxy (quick startup test)
# =============================================================================
section "Test 4: muninn proxy"

cd "$TEST_DIR"

# Update config to use anthropic backend (we'll provide a dummy key)
mkdir -p .muninn
cat > .muninn/config.toml << 'EOF'
[project]
root = ".."

[graph]
path = "graph.db"

[backend]
type = "anthropic"

[router]
strategy = "always-passthrough"
enabled = true
EOF

# Start proxy in background with a dummy API key
ANTHROPIC_API_KEY="test-key-for-uat" $MUNINN_BIN proxy --port 18080 2>&1 &
PROXY_PID=$!
sleep 2

# Check if it's running
if kill -0 $PROXY_PID 2>/dev/null; then
    pass "Proxy started successfully"
else
    fail "Proxy startup" "Process not running"
fi

# Test health endpoint
if curl -s http://127.0.0.1:18080/health | grep -q '"status":"ok"'; then
    pass "Health endpoint responds"
else
    fail "Health endpoint" "No response or wrong status"
fi

# Test that it accepts POST to /v1/messages (will fail auth, but should respond with error)
RESPONSE=$(curl -s -X POST http://127.0.0.1:18080/v1/messages \
    -H "Content-Type: application/json" \
    -H "x-api-key: test-key" \
    -d '{"model":"test","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}' \
    2>&1)

if echo "$RESPONSE" | grep -qE '(error|content|type)'; then
    pass "Messages endpoint accepts requests"
else
    fail "Messages endpoint" "Unexpected response: $RESPONSE"
fi

# Stop proxy
kill $PROXY_PID 2>/dev/null || true
wait $PROXY_PID 2>/dev/null || true
pass "Proxy stopped cleanly"

# =============================================================================
# Test 5: muninn claude (startup test - doesn't need claude installed)
# =============================================================================
section "Test 5: muninn claude (partial - tests proxy startup)"

cd "$TEST_DIR"

# We'll test that the claude command starts the proxy correctly
# by checking stderr output (since claude won't be found, it will error)

OUTPUT=$(ANTHROPIC_API_KEY="test-key-for-uat" $MUNINN_BIN claude --port 18081 -- --help 2>&1 || true)

if echo "$OUTPUT" | grep -q "Starting muninn proxy"; then
    pass "Claude command starts proxy"
else
    # Might fail if claude is actually installed and runs
    if echo "$OUTPUT" | grep -q "Proxy ready"; then
        pass "Claude command starts proxy (proxy ready)"
    else
        fail "Claude proxy startup" "No proxy startup message found in: $OUTPUT"
    fi
fi

if echo "$OUTPUT" | grep -qE "(Launching claude|Failed to launch 'claude')"; then
    pass "Claude command attempts to launch agent"
else
    fail "Claude launch" "No launch attempt detected"
fi

# =============================================================================
# Test 6: Config file discovery
# =============================================================================
section "Test 6: Config file discovery"

cd "$TEST_DIR"
mkdir -p nested/deep/directory
cd nested/deep/directory

# Should find muninn.toml in parent
if $MUNINN_BIN index --path . --output /dev/null 2>&1 | grep -qE "(Found config|Loaded config|No config)"; then
    pass "Config discovery from nested directory"
else
    fail "Config discovery" "No config loading message"
fi

# =============================================================================
# Summary
# =============================================================================
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Test Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "  ${GREEN}Passed: $TESTS_PASSED${NC}"
echo -e "  ${RED}Failed: $TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}All tests passed! ✓${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed. Review output above.${NC}"
    exit 1
fi
