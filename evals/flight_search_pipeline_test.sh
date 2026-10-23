#!/usr/bin/env bash
# Real end-to-end eval of the whole learning pipeline: a live claude -p --chrome
# session searches flights on makemytrip.com, driving recall/capture hooks -> real
# RabbitMQ -> the real `learn` consumer -> live JEV calls -> field_pairs ->
# facts/tasks -> real Qdrant + Redis. Prints exactly what ends up stored in Qdrant
# (user_facts, user_routines) and Redis (tasks:counts) afterward.
#
# Safety: the agent is explicitly told to stop at viewing the cheapest matching
# flight - never to enter payment or personal/traveler details or complete a
# purchase. This script does NOT clean up after itself - the data is left in place
# for inspection.
#
# Requires: reachable Redis/RabbitMQ/Qdrant (docker compose up), OPENROUTER_API_KEY
# in .env, an authenticated claude CLI with claude-in-chrome paired.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RECALL_BIN="$REPO_ROOT/target/debug/recall"
CAPTURE_BIN="$REPO_ROOT/target/debug/capture"
SESSION_ID="$(python3 -c 'import uuid; print(uuid.uuid4())')"
LEARN_LOG="$(mktemp -t learn_consumer_eval.XXXXXX.log)"

set -a
source "$REPO_ROOT/.env"
set +a

echo "==> building recall, capture, rabbitmq tools"
(cd "$REPO_ROOT" && cargo build --quiet -p recall -p capture -p rabbitmq)

echo "==> provisioning rabbitmq topology (idempotent)"
(cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-setup)

echo "==> starting the real learn consumer in the background (log: $LEARN_LOG)"
# Invoke the venv's python directly (not "uv run python ...") - uv run spawns a
# wrapper process around a separate python child, so $! only captures the wrapper's
# PID and killing it later leaves the real consumer orphaned and still running.
# "exec" as the subshell's last command replaces the subshell with python outright,
# so $! is guaranteed to be the real consumer's PID, not a wrapper around it - this
# alone wasn't enough last time (plain `kill` left 9 of these running for over an
# hour, apparently not dying from SIGTERM promptly), so the trap below now also
# force-kills with -9.
# -u disables stdout buffering so the log actually fills in while the process runs,
# instead of sitting empty until an internal buffer flushes.
(cd "$REPO_ROOT/learn" && exec "$REPO_ROOT/learn/.venv/bin/python" -u -m learn.consumer) > "$LEARN_LOG" 2>&1 &
CONSUMER_PID=$!
trap 'kill -9 "$CONSUMER_PID" 2>/dev/null || true' EXIT
sleep 2

TEST_DIR="$(mktemp -d)"

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

PROMPT="go to airindia.com and look up flights on 25th october mumbai to delhi. i am a student and need a flight of early morning and only direct flights. give me a cheap one. This is a lookup only - do not log in or enter any personal/payment details, just find and report the flight. Important: use read_page and find to locate elements and click/type by ref - do not take screenshots and do not click by raw coordinates."

echo "==> running claude -p --chrome (session=$SESSION_ID): flight search on airindia.com"
(
  cd "$TEST_DIR"
  claude -p "$PROMPT" \
    --settings "$TEST_DIR/settings.json" \
    --chrome \
    --session-id "$SESSION_ID" \
    --dangerously-skip-permissions \
    --output-format text \
    < /dev/null \
    > "$TEST_DIR/claude_output.txt" 2>"$TEST_DIR/claude_stderr.log"
)

echo "--- agent's final message ---"
cat "$TEST_DIR/claude_output.txt"

echo "==> waiting for the learn consumer to finish (two real JEV calls happen after Stop fires)"
# macOS has no `timeout` command by default - poll manually instead, capped at 90s.
waited=0
while ! grep -q "jev decision for session=$SESSION_ID" "$LEARN_LOG" && [ "$waited" -lt 90 ]; do
  sleep 1
  waited=$((waited + 1))
done
if [ "$waited" -ge 90 ]; then
  echo "WARN: didn't see a jev decision logged within 90s - checking stored state anyway"
fi
sleep 5  # let judge_result's own JEV classify_pairs call (fires after the decision log line) finish too

echo ""
echo "=== learn consumer log (this session only) ==="
grep "session=$SESSION_ID\|session_id.*$SESSION_ID" "$LEARN_LOG" || cat "$LEARN_LOG"

echo ""
echo "=== Qdrant: user_facts ==="
(cd "$REPO_ROOT/learn" && uv run python -c "
from learn import facts
if facts._client.collection_exists(facts.COLLECTION):
    points, _ = facts._client.scroll(collection_name=facts.COLLECTION, limit=1000, with_payload=True, with_vectors=False)
    for p in points:
        print(p.payload)
    if not points:
        print('(collection exists, empty)')
else:
    print('(collection does not exist - nothing ever classified as personal)')
" 2>&1 | grep -v "^Loading\|^Warning\|torch/distributed")

echo ""
echo "=== Qdrant: user_routines ==="
(cd "$REPO_ROOT/learn" && uv run python -c "
from learn import tasks
if tasks._qdrant.collection_exists(tasks.COLLECTION):
    points, _ = tasks._qdrant.scroll(collection_name=tasks.COLLECTION, limit=1000, with_payload=True, with_vectors=False)
    for p in points:
        print(p.payload)
    if not points:
        print('(collection exists, empty)')
else:
    print('(collection does not exist - nothing ever promoted)')
" 2>&1 | grep -v "^Loading\|^Warning\|torch/distributed")

echo ""
echo "=== Redis: tasks:counts (all task counters, no TTL) ==="
(cd "$REPO_ROOT/learn" && uv run python -c "
from learn import tasks
counts = tasks._redis.hgetall(tasks.TASKS_KEY)
print(dict(counts) if counts else '(empty)')
" 2>&1 | grep -v "^Loading\|^Warning\|torch/distributed")

echo ""
echo "DONE: session=$SESSION_ID - data left in place for inspection (not cleaned up by this script)"
