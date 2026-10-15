#!/usr/bin/env bash
# Integration test: a real browser task, end to end, through the real RabbitMQ broker.
#
# Runs one claude -p --chrome session with BOTH recall (UserPromptSubmit, PreToolUse)
# and capture (PostToolUse) registered as hooks, driving a real "look up a product
# price on Amazon" task. Verifies not that the code *claims* to publish, but that the
# messages actually landed in the broker: computes which queue this session_id hashes
# to (rabbitmq-queue-for), drains it for real (rabbitmq-drain), and asserts every
# expected message kind (prompt/steps/page.snapshot) is really there, all correctly
# tagged with this session's own session_id - not just that *a* message showed up
# somewhere.
#
# Requires: a reachable RabbitMQ (RABBITMQ_URL, same var capture/recall use - defaults
# to amqp://guest:guest@127.0.0.1:5672/%2f), an authenticated claude CLI with
# claude-in-chrome paired, and jq. This script provisions the topology itself
# (rabbitmq-setup is idempotent) before running.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RECALL_BIN="$REPO_ROOT/target/debug/recall"
CAPTURE_BIN="$REPO_ROOT/target/debug/capture"
SESSION_ID="$(python3 -c 'import uuid; print(uuid.uuid4())')"

echo "==> building recall, capture, rabbitmq tools"
(cd "$REPO_ROOT" && cargo build --quiet -p recall -p capture -p rabbitmq)

echo "==> provisioning rabbitmq topology (idempotent)"
(cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-setup)

TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT

cat > "$TEST_DIR/settings.json" <<EOF
{
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "$RECALL_BIN" } ] }
    ],
    "PreToolUse": [
      { "matcher": "mcp__claude-in-chrome__.*", "hooks": [ { "type": "command", "command": "$RECALL_BIN" } ] }
    ],
    "PostToolUse": [
      { "matcher": "mcp__claude-in-chrome__.*", "hooks": [ { "type": "command", "command": "$CAPTURE_BIN" } ] }
    ]
  }
}
EOF

echo "==> running claude -p --chrome (session=$SESSION_ID): iPhone price on amazon.in"
(
  cd "$TEST_DIR"
  claude -p \
    "Create a fresh tab (tabs_create_mcp), then go to https://amazon.in and find out the price of the new iPhone. Use find or read_page to locate things on the page - no screenshots or get_page_text, and no reusing an existing tab even if one is already open. Close the tab when done." \
    --settings "$TEST_DIR/settings.json" \
    --chrome \
    --session-id "$SESSION_ID" \
    --dangerously-skip-permissions \
    --output-format text \
    < /dev/null \
    > "$TEST_DIR/claude_output.txt" 2>"$TEST_DIR/claude_stderr.log"
)

QUEUE_NUM=$(cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-queue-for -- "$SESSION_ID")
echo "==> session $SESSION_ID routes to worker.$QUEUE_NUM - draining it"

MESSAGES_FILE="$TEST_DIR/messages.jsonl"
(cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-drain -- "worker.$QUEUE_NUM") > "$MESSAGES_FILE"

if [ ! -s "$MESSAGES_FILE" ]; then
  echo "FAIL: no messages found in worker.$QUEUE_NUM at all"
  echo "--- claude's own output, for debugging ---"
  cat "$TEST_DIR/claude_output.txt"
  exit 1
fi

fail=0

# Every message actually found must belong to this session - proves the routing key
# (and the queue binding) is doing its job, not "some queue happened to have messages."
other_session_count=$(jq -s --arg sid "$SESSION_ID" '[.[] | select(.session_id != $sid)] | length' "$MESSAGES_FILE")
if [ "$other_session_count" -gt 0 ]; then
  echo "FAIL: $other_session_count message(s) in worker.$QUEUE_NUM belong to a different session"
  fail=1
fi

for kind in prompt steps page.snapshot; do
  count=$(jq -s --arg k "$kind" --arg sid "$SESSION_ID" '[.[] | select(.kind == $k and .session_id == $sid)] | length' "$MESSAGES_FILE")
  echo "  kind=$kind count=$count"
  if [ "$count" -eq 0 ]; then
    echo "FAIL: no '$kind' message found for this session"
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "--- all messages found in worker.$QUEUE_NUM, for debugging ---"
  jq . "$MESSAGES_FILE"
  echo "--- claude's own output, for debugging ---"
  cat "$TEST_DIR/claude_output.txt"
  exit 1
fi

echo "PASS: prompt, steps, and page.snapshot messages all really landed in worker.$QUEUE_NUM for session $SESSION_ID"
