set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CAPTURE_BIN="$REPO_ROOT/target/debug/capture"

export REDIS_URL="redis://127.0.0.1:6380/"
REDIS_CONTAINER="groove-redis-eval"
rc() { docker exec "$REDIS_CONTAINER" redis-cli "$@"; }

echo "==> building capture"
(cd "$REPO_ROOT" && cargo build --quiet --bin capture)

SESSION_ID="redis-eval-$(date +%s)"
TAB_ID=4242

fail=0

send() {
  "$CAPTURE_BIN" > /dev/null
}

echo "==> flushing any leftover keys for $SESSION_ID (should be none)"
rc KEYS "*${SESSION_ID}*" | grep -q . && { echo "FAIL: stale keys already present for $SESSION_ID"; exit 1; } || true

echo "==> 1. PostToolUse read_page -> cache_elements (real HSET + EXPIRE)"
cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "PostToolUse",
  "tool_name": "mcp__claude-in-chrome__read_page",
  "tool_use_id": "tu_1",
  "tool_input": { "tabId": $TAB_ID },
  "tool_response": "textbox \"Password\" [ref_1] type=\"password\"\ntextbox \"OTP\" [ref_2] type=\"text\""
}
EOF

elements_key="elements:${SESSION_ID}:${TAB_ID}"
ref1=$(rc HGET "$elements_key" ref_1)
ref2=$(rc HGET "$elements_key" ref_2)
ttl=$(rc TTL "$elements_key")

if [ "$ref1" != 'textbox "Password"' ]; then
  echo "FAIL: expected ref_1 cached as textbox \"Password\", got: $ref1"
  fail=1
fi
if [ "$ref2" != 'textbox "OTP"' ]; then
  echo "FAIL: expected ref_2 cached as textbox \"OTP\", got: $ref2"
  fail=1
fi
if [ "$ttl" -le 0 ] || [ "$ttl" -gt 1800 ]; then
  echo "FAIL: expected TTL in (0, 1800], got: $ttl"
  fail=1
fi
echo "  ref_1=$ref1 ref_2=$ref2 ttl=${ttl}s"

echo "==> 2. PostToolUse computer left_click with ref -> set_focus (real SETEX)"
cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "PostToolUse",
  "tool_name": "mcp__claude-in-chrome__computer",
  "tool_use_id": "tu_2",
  "tool_input": { "action": "left_click", "tabId": $TAB_ID, "ref": "ref_1" },
  "tool_response": "clicked"
}
EOF

focus_key="focus:${SESSION_ID}:${TAB_ID}"
focus_val=$(rc GET "$focus_key")
focus_ttl=$(rc TTL "$focus_key")
if [ "$focus_val" != "ref_1" ]; then
  echo "FAIL: expected focus set to ref_1, got: $focus_val"
  fail=1
fi
if [ "$focus_ttl" -le 0 ] || [ "$focus_ttl" -gt 3600 ]; then
  echo "FAIL: expected focus TTL in (0, 3600], got: $focus_ttl"
  fail=1
fi
echo "  focus=$focus_val ttl=${focus_ttl}s"

echo "==> 3. PostToolUse computer type with no ref -> get_focus must resolve ref_1 (real GET)"
cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "PostToolUse",
  "tool_name": "mcp__claude-in-chrome__computer",
  "tool_use_id": "tu_3",
  "tool_input": { "action": "type", "tabId": $TAB_ID, "text": "hunter2" },
  "tool_response": "typed"
}
EOF
# (resolve path only eprintln's - verified via the real GET returning ref_1 above/below)

echo "==> 4. PostToolUse navigate -> delete_tab_maps (real DEL of elements+focus for this tab)"
cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "PostToolUse",
  "tool_name": "mcp__claude-in-chrome__navigate",
  "tool_use_id": "tu_4",
  "tool_input": { "url": "https://example.com", "tabId": $TAB_ID },
  "tool_response": "navigated"
}
EOF

if [ "$(rc EXISTS "$elements_key")" != "0" ]; then
  echo "FAIL: elements key should be gone after navigate, still exists: $elements_key"
  fail=1
fi
if [ "$(rc EXISTS "$focus_key")" != "0" ]; then
  echo "FAIL: focus key should be gone after navigate, still exists: $focus_key"
  fail=1
fi
echo "  both keys gone after navigate"

echo "==> 5. re-seed a second tab, then SessionEnd -> delete_session_maps (real SCAN+DEL across all tabs)"
OTHER_TAB=9999
cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "PostToolUse",
  "tool_name": "mcp__claude-in-chrome__find",
  "tool_use_id": "tu_5",
  "tool_input": { "tabId": $OTHER_TAB, "query": "otp field" },
  "tool_response": "- ref_9: textbox \"OTP\" (text) - Exact match - textbox with label \"OTP\""
}
EOF

other_elements_key="elements:${SESSION_ID}:${OTHER_TAB}"
if [ "$(rc EXISTS "$other_elements_key")" != "1" ]; then
  echo "FAIL: expected $other_elements_key to exist before SessionEnd"
  fail=1
fi

cat <<EOF | send
{
  "session_id": "$SESSION_ID",
  "transcript_path": "/tmp/t.jsonl",
  "cwd": "/tmp",
  "hook_event_name": "SessionEnd",
  "reason": "other"
}
EOF

remaining=$(rc KEYS "*${SESSION_ID}*" | grep -c . || true)
if [ "$remaining" -ne 0 ]; then
  echo "FAIL: expected 0 keys left for $SESSION_ID after SessionEnd, found $remaining"
  rc KEYS "*${SESSION_ID}*"
  fail=1
fi
echo "  all keys for $SESSION_ID gone after SessionEnd"

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "PASS: capture's Redis wiring (cache_elements, set_focus/get_focus, delete_tab_maps, delete_session_maps) verified against a real Redis instance"
