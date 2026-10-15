
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RECALL_BIN="$REPO_ROOT/target/debug/recall"
SECRET="sk-testsecretvalueabcdefghijklmnop"
TEST_HINT="groove-test-hint-$(date +%s)"
PORT=8743

echo "==> building recall"
(cd "$REPO_ROOT" && cargo build --quiet --bin recall)

TEST_DIR="$(mktemp -d)"
SERVER_PID=""
cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  rm -rf "$TEST_DIR"
}
trap cleanup EXIT

cat > "$TEST_DIR/login_test.html" <<'HTML'
<!DOCTYPE html>
<html>
<head><title>Test Login</title></head>
<body>
  <h1>Sign in</h1>
  <form id="login-form">
    <label for="username">Username</label>
    <input id="username" type="text" name="username" autocomplete="username">

    <label for="password">Password</label>
    <input id="password" type="password" name="password" autocomplete="current-password">

    <button type="submit" id="submit-btn">Log in</button>
  </form>
  <p id="status"></p>
  <script>
    document.getElementById('login-form').addEventListener('submit', function (e) {
      e.preventDefault();
      document.getElementById('status').textContent = 'Logged in as ' + document.getElementById('username').value;
    });
  </script>
</body>
</html>
HTML

cat > "$TEST_DIR/settings.json" <<EOF
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "mcp__claude-in-chrome__.*",
        "hooks": [
          { "type": "command", "command": "$RECALL_BIN" }
        ]
      }
    ]
  }
}
EOF

echo "==> serving test login page on :$PORT"
(cd "$TEST_DIR" && python3 -m http.server "$PORT" > /dev/null 2>&1) &
SERVER_PID=$!
sleep 1

OUTPUT_FILE="$TEST_DIR/stream.jsonl"

echo "==> running claude -p --chrome with recall registered as the PreToolUse hook"
(
  cd "$TEST_DIR"
  GROOVE_TEST_HINT="$TEST_HINT" claude -p \
    "Check out this website at http://localhost:${PORT}/login_test.html -- these are the login creds: username is alice, password is ${SECRET}. Please log in. Use find or read_page to locate the username and password fields and form_input to fill them in directly - no screenshots needed, this is a simple static form." \
    --settings "$TEST_DIR/settings.json" \
    --chrome \
    --dangerously-skip-permissions \
    --output-format stream-json \
    --include-hook-events \
    --verbose \
    < /dev/null \
    > "$OUTPUT_FILE"
)

# Every PreToolUse hook_response for a claude-in-chrome tool call, one JSON object per
# line (jq -c). Written to a file, not a variable - avoids a second layer of shell
# reinterpretation on top of the printf discipline below.
RESPONSES_FILE="$TEST_DIR/chrome_hook_responses.jsonl"
jq -c 'select(.subtype == "hook_response" and (.hook_name | startswith("PreToolUse:mcp__claude-in-chrome__")))' \
  "$OUTPUT_FILE" > "$RESPONSES_FILE"

if [ ! -s "$RESPONSES_FILE" ]; then
  echo "FAIL: no mcp__claude-in-chrome__* hook_response found in the hook event stream"
  exit 1
fi

fail=0
saw_redaction=0

while IFS= read -r response; do
  stdout_field=$(printf '%s\n' "$response" | jq -r '.stdout // empty')
  stderr_field=$(printf '%s\n' "$response" | jq -r '.stderr // empty')
  exit_code=$(printf '%s\n' "$response" | jq -r '.exit_code')
  hook_name=$(printf '%s\n' "$response" | jq -r '.hook_name')

  # 1. Redaction: the raw secret must never appear in anything recall itself emitted,
  # on any call, and recall's own "redacted N sensitive value(s): ..." summary line must
  # show up on at least one. (recall only logs that summary, not a payload dump, so
  # that's the only marker available here - see shared/privacy.rs.)
  if printf '%s\n%s\n' "$stdout_field" "$stderr_field" | grep -qF "$SECRET"; then
    echo "FAIL: raw secret leaked into recall's own output for $hook_name"
    fail=1
  fi
  if printf '%s\n' "$stderr_field" | grep -q "redacted"; then
    saw_redaction=1
  fi

  if [ "$exit_code" != "0" ]; then
    echo "FAIL: $hook_name exited non-zero ($exit_code)"
    fail=1
  fi

  # 2. Hint delivery: find_hint ignores tool_name/tool_input entirely when
  # GROOVE_TEST_HINT is set (see recall/hint.rs) - it's unconditional, so every single
  # PreToolUse call, for every tool, must carry it. Not "at least one": a future bug that
  # scopes find_hint to only some tool types should fail this test, not slip through
  # because some other call happened to carry it.
  returned_hint=$(printf '%s\n' "$stdout_field" | jq -r '.hookSpecificOutput.additionalContext // empty')
  if [ "$returned_hint" != "$TEST_HINT" ]; then
    echo "FAIL: hint missing or wrong on $hook_name"
    echo "  expected: $TEST_HINT"
    echo "       got: $returned_hint"
    fail=1
  fi
done < "$RESPONSES_FILE"

if [ "$saw_redaction" -ne 1 ]; then
  echo "FAIL: no claude-in-chrome call showed the password being redacted"
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "--- all claude-in-chrome hook_response events, for debugging ---"
  jq . "$RESPONSES_FILE"
  exit 1
fi

echo "PASS: password redacted before leaving recall, hint delivered correctly, on a real claude-in-chrome call"
