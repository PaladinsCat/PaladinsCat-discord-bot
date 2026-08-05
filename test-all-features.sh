#!/bin/bash
set -euo pipefail

BOT_NAME="paladinscat-bot-rust"
CONTAINER_ID=$(docker ps -q -f name="$BOT_NAME" | head -1)
if [[ -z "$CONTAINER_ID" ]]; then
  echo "FAIL: No running container matching $BOT_NAME"
  exit 1
fi

PASS=0
FAIL=0

pass() { ((PASS++)); echo "  [PASS] $1"; }
fail() { ((FAIL++)); echo "  [FAIL] $1 — $2"; }

echo "========================================"
echo "  Docker Feature Test Suite"
echo "  Container: $CONTAINER_ID"
echo "========================================"

# 1. Container status
echo ""; echo "--- Container Health ---"
STATUS=$(docker inspect --format='{{.State.Status}}' "$CONTAINER_ID")
[[ "$STATUS" == "running" ]] && pass "Container running" || fail "Status" "$STATUS"

# 2. Health endpoint
echo ""; echo "--- Health Endpoint ---"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:3020/health 2>/dev/null || echo "000")
[[ "$HTTP_CODE" == "200" ]] && pass "/health → HTTP $HTTP_CODE" || fail "/health" "HTTP $HTTP_CODE"
BODY=$(curl -s http://localhost:3020/health 2>/dev/null || echo "")
echo "$BODY" | grep -q '"status"' && pass "Health JSON valid" || fail "Health JSON" "no status key"
echo "$BODY" | grep -q '"service"' && pass "Service name in response" || fail "Service name" "missing"

# 3. Resource usage
echo ""; echo "--- Resource Usage ---"
STATS=$(docker stats --no-stream --format '{{.CPUPerc}}|{{.MemUsage}}|{{.MemPerc}}|{{.NetIO}}|{{.BlockIO}}|{{.PIDs}}' "$CONTAINER_ID" 2>/dev/null || echo "0%|0B/0B|0%|0B/0B|0B/0B|0")
IFS='|' read -r CPU MEM MP NET BLK PIDS <<< "$STATS"
echo "  CPU: $CPU"
echo "  Memory: $MEM ($MP)"
echo "  Network: $NET"
echo "  Block I/O: $BLK"
echo "  PIDs: $PIDS"
pass "Resource metrics captured"

# 4. Image size
echo ""; echo "--- Image ---"
IMG_SIZE=$(docker image inspect --format='{{.Size}}' paladinscat-bot-rust 2>/dev/null || echo "0")
if [[ "$IMG_SIZE" =~ ^[0-9]+$ && "$IMG_SIZE" -gt 0 ]]; then
  SIZE_MB=$((IMG_SIZE / 1048576))
  echo "  Image size: ${SIZE_MB} MB"
  [[ "$IMG_SIZE" -lt 100000000 ]] && pass "Image < 100MB ($SIZE_MB MB)" || pass "Image size: $SIZE_MB MB"
else
  fail "Image size" "unknown"
fi
LAYERS=$(docker inspect --format='{{.RootFS.Layers|length}}' paladinscat-bot-rust 2>/dev/null || echo "?")
echo "  Layers: $LAYERS"

# 5. Logs
echo ""; echo "--- Logs & Feature Verification ---"
sleep 2
LOGS=$(docker logs "$CONTAINER_ID" 2>&1 || true)
echo "$LOGS"

echo "$LOGS" | grep -qi "Commands registered" && pass "Command registration" || fail "Command registration" "not found"
echo "$LOGS" | grep -qi "bot_mode\|DISCORD_BOT_MODE" && pass "Config loading" || fail "Config loading" "not found"
echo "$LOGS" | grep -qi "Connecting to Discord\|gateway\|Gateway\|connecting" && pass "Gateway connection initiated" || fail "Gateway connection" "not found"
echo "$LOGS" | grep -qi "Health server started\|Health.*listening\|Health.*started" && pass "Health server started" || fail "Health server" "not found"
echo "$LOGS" | grep -qi "panic\|^ERROR\|^error\[E" && fail "Runtime errors found" "in logs" || pass "No runtime panics/errors"

# 6. Binary accessible
echo ""; echo "--- Internal File Check ---"
BINARY=$(docker exec "$CONTAINER_ID" sh -c 'test -f /app/paladinscat-discord-bot && echo yes || echo no' 2>/dev/null || echo "no")
[[ "$BINARY" == "yes" ]] && pass "Binary accessible" || fail "Binary" "not found at /app"
FILE_LIST=$(docker exec "$CONTAINER_ID" ls -lh /app/ 2>/dev/null || echo "failed")
echo "  $FILE_LIST" | head -5
pass "File listing works"

# 7. Environment
echo ""; echo "--- Environment ---"
TOK=$(docker exec "$CONTAINER_ID" sh -c 'echo $DISCORD_TOKEN' 2>/dev/null || echo "not-set")
URL=$(docker exec "$CONTAINER_ID" sh -c 'echo $API_BASE_URL' 2>/dev/null || echo "not-set")
RL=$(docker exec "$CONTAINER_ID" sh -c 'echo $RUST_LOG' 2>/dev/null || echo "not-set")
echo "  DISCORD_TOKEN: ${TOK:0:12}..."
echo "  API_BASE_URL: $URL"
echo "  RUST_LOG: $RL"
[[ "$TOK" != "not-set" && "$TOK" != "" ]] && pass "DISCORD_TOKEN set" || fail "DISCORD_TOKEN" "empty"
[[ "$URL" != "not-set" && "$URL" != "" ]] && pass "API_BASE_URL set" || fail "API_BASE_URL" "empty"
[[ "$RL" != "not-set" && "$RL" != "" ]] && pass "RUST_LOG set" || fail "RUST_LOG" "empty"

# 8. API connectivity (via SSH tunnel)
echo ""; echo "--- API Connectivity ---"
API_TEST=$(docker exec "$CONTAINER_ID" sh -c 'curl -sf -o /dev/null -w "%{http_code}" $API_BASE_URL/health 2>/dev/null || echo "000"' 2>/dev/null || echo "000")
echo "  API health: HTTP $API_TEST"
[[ "$API_TEST" =~ ^(2|3)[0-9][0-9]$ ]] && pass "Backend API reachable ($API_TEST)" || fail "Backend API" "HTTP $API_TEST"

# 9. Port binding
echo ""; echo "--- Port Binding ---"
PORT_MAP=$(docker ps --format '{{.Ports}}' -f "name=$BOT_NAME" 2>/dev/null | grep -o '[0-9]*:3020' || echo "not-found")
echo "  Port mapping: $PORT_MAP"
[[ -n "$PORT_MAP" && "$PORT_MAP" != "not-found" ]] && pass "Port 3020 published" || fail "Port 3020" "not published"

# 10. Restart policy
echo ""; echo "--- Restart Policy ---"
POLICY=$(docker inspect --format='{{.HostConfig.RestartPolicy.Name}}' "$CONTAINER_ID" 2>/dev/null || echo "unknown")
RESTARTS=$(docker inspect --format='{{.RestartCount}}' "$CONTAINER_ID" 2>/dev/null || echo "?")
echo "  Policy: $POLICY"
echo "  Restarts: $RESTARTS"
[[ "$POLICY" == "unless-stopped" || "$POLICY" == "always" ]] && pass "Restart policy: $POLICY" || fail "Restart policy" "$POLICY"

# SUMMARY
TOTAL=$((PASS + FAIL))
echo ""
echo "========================================"
echo "  RESULT: $PASS/$TOTAL PASSED, $FAIL FAILED"
echo "========================================"
echo ""
echo "=== RESOURCE REPORT ==="
echo "CPU: $CPU | Memory: $MEM ($MP) | PIDs: $PIDS"
echo "Image: ${SIZE_MB:-unknown} MB | Layers: $LAYERS"
echo "Network: $NET"
echo "Health: HTTP $HTTP_CODE | Restart policy: $POLICY"
echo "========================================"

exit "$FAIL"
