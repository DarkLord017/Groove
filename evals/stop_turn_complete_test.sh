#!/usr/bin/env bash
# Integration test: proves what capture actually publishes to RabbitMQ when a real
# Claude Code turn ends (the Stop hook - "run.end" in the system design, "turn.complete"
# as the actual RabbitMessageKind in code).
#
# Runs one real claude -p --chrome session with capture registered as the PostToolUse
# AND Stop hook (plus recall on PreToolUse, so the run looks like a normal one) driving
# a quick, deterministic browser task. Verifies not that the code *claims* to publish a
# turn.complete message, but that it actually landed in the broker: computes which queue
# this session_id hashes to (rabbitmq-queue-for), drains it for real (rabbitmq-drain),
# and asserts a turn.complete message is really there for this session_id, with a
# non-empty last_assistant_message - then prints the exact message.
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
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "$CAPTURE_BIN" } ] }
    ]
  }
}
EOF

echo "==> running claude -p --chrome (session=$SESSION_ID): quick, deterministic browser task"
(
  cd "$TEST_DIR"
  claude -p \
    "Create a fresh tab (tabs_create_mcp), navigate to https://example.com, use read_page to confirm the page loaded, then close the tab. Reply with the page title you found." \
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

TURN_COMPLETE=$(jq -s --arg sid "$SESSION_ID" '[.[] | select(.kind == "turn.complete" and .session_id == $sid)]' "$MESSAGES_FILE")
TURN_COMPLETE_COUNT=$(printf '%s\n' "$TURN_COMPLETE" | jq 'length')

fail=0

if [ "$TURN_COMPLETE_COUNT" -eq 0 ]; then
  echo "FAIL: no turn.complete message found for this session in worker.$QUEUE_NUM"
  fail=1
else
  LAST_MESSAGE=$(printf '%s\n' "$TURN_COMPLETE" | jq -r '.[0].payload.last_assistant_message // empty')
  if [ -z "$LAST_MESSAGE" ]; then
    echo "FAIL: turn.complete message has an empty last_assistant_message"
    fail=1
  fi
fi

if [ "$fail" -ne 0 ]; then
  echo "--- all messages found in worker.$QUEUE_NUM, for debugging ---"
  jq . "$MESSAGES_FILE"
  echo "--- claude's own output, for debugging ---"
  cat "$TEST_DIR/claude_output.txt"
  exit 1
fi

echo "PASS: real Stop hook fired, capture published a turn.complete message for session $SESSION_ID"
echo "--- the actual message, as it landed on RabbitMQ ---"
printf '%s\n' "$TURN_COMPLETE" | jq '.[0]'
