set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RECALL_BIN="$REPO_ROOT/target/debug/recall"
CAPTURE_BIN="$REPO_ROOT/target/debug/capture"

echo "==> building recall, capture, rabbitmq tools"
(cd "$REPO_ROOT" && cargo build --quiet -p recall -p capture -p rabbitmq)

echo "==> provisioning rabbitmq topology (idempotent)"
(cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-setup)

queue_for() {
  (cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-queue-for -- "$1")
}

SHOPPING_SESSION=""
FLIGHT_SESSION=""
SHOPPING_QUEUE=""
FLIGHT_QUEUE=""

echo "==> finding two session_ids that hash to different queues"
for attempt in 1 2 3 4 5; do
  candidate_a="$(python3 -c 'import uuid; print(uuid.uuid4())')"
  candidate_b="$(python3 -c 'import uuid; print(uuid.uuid4())')"
  q_a=$(queue_for "$candidate_a")
  q_b=$(queue_for "$candidate_b")
  if [ "$q_a" != "$q_b" ]; then
    SHOPPING_SESSION="$candidate_a"
    FLIGHT_SESSION="$candidate_b"
    SHOPPING_QUEUE="$q_a"
    FLIGHT_QUEUE="$q_b"
    break
  fi
done

if [ -z "$SHOPPING_QUEUE" ]; then
  echo "FAIL: could not find two session_ids hashing to different queues in 5 tries (suspiciously unlucky - check QUEUE_COUNT)"
  exit 1
fi

echo "  shopping session $SHOPPING_SESSION -> worker.$SHOPPING_QUEUE"
echo "  flight   session $FLIGHT_SESSION -> worker.$FLIGHT_QUEUE"

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

run_session() {
  local session_id="$1"
  local prompt="$2"
  local out_file="$3"
  (
    cd "$TEST_DIR"
    claude -p \
      "$prompt" \
      --settings "$TEST_DIR/settings.json" \
      --chrome \
      --session-id "$session_id" \
      --dangerously-skip-permissions \
      --output-format text \
      < /dev/null \
      > "$out_file" 2>&1
  )
}

echo "==> running shopping and flight sessions in parallel ($SHOPPING_SESSION, $FLIGHT_SESSION)"
run_session "$SHOPPING_SESSION" \
  "Create a fresh tab (tabs_create_mcp), then go to https://amazon.in and find out the price of the new iPhone. Use find or read_page to locate things - no screenshots or get_page_text, and no reusing an existing tab even if one is already open. Close the tab when done." \
  "$TEST_DIR/shopping_output.txt" &
shopping_pid=$!

run_session "$FLIGHT_SESSION" \
  "Create a fresh tab (tabs_create_mcp), then go to https://www.google.com/travel/flights and look for flights from New York to San Francisco. Use find or read_page to locate things - no screenshots or get_page_text, and no reusing an existing tab even if one is already open. Close the tab when done." \
  "$TEST_DIR/flight_output.txt" &
flight_pid=$!

# set -e would otherwise abort at the first non-zero wait, potentially before the
# other background job is even waited on - collect both exit codes first instead.
shopping_exit=0
wait "$shopping_pid" || shopping_exit=$?
flight_exit=0
wait "$flight_pid" || flight_exit=$?

if [ "$shopping_exit" -ne 0 ]; then
  echo "FAIL: shopping session's claude -p exited non-zero ($shopping_exit)"
  fail_run=1
fi
if [ "$flight_exit" -ne 0 ]; then
  echo "FAIL: flight session's claude -p exited non-zero ($flight_exit)"
  fail_run=1
fi
if [ "${fail_run:-0}" -ne 0 ]; then
  echo "--- shopping output ---"; cat "$TEST_DIR/shopping_output.txt"
  echo "--- flight output ---"; cat "$TEST_DIR/flight_output.txt"
  exit 1
fi

SHOPPING_MESSAGES="$TEST_DIR/shopping_messages.jsonl"
FLIGHT_MESSAGES="$TEST_DIR/flight_messages.jsonl"

if [ "$SHOPPING_QUEUE" = "$FLIGHT_QUEUE" ]; then
  echo "==> both sessions share worker.$SHOPPING_QUEUE (same queue, different sessions - allowed) - draining once"
  (cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-drain -- "worker.$SHOPPING_QUEUE") > "$TEST_DIR/shared_messages.jsonl"
  jq -c --arg sid "$SHOPPING_SESSION" 'select(.session_id == $sid)' "$TEST_DIR/shared_messages.jsonl" > "$SHOPPING_MESSAGES" || true
  jq -c --arg sid "$FLIGHT_SESSION" 'select(.session_id == $sid)' "$TEST_DIR/shared_messages.jsonl" > "$FLIGHT_MESSAGES" || true
else
  echo "==> draining worker.$SHOPPING_QUEUE (shopping) and worker.$FLIGHT_QUEUE (flight) separately"
  (cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-drain -- "worker.$SHOPPING_QUEUE") > "$SHOPPING_MESSAGES"
  (cd "$REPO_ROOT" && cargo run --quiet -p rabbitmq --bin rabbitmq-drain -- "worker.$FLIGHT_QUEUE") > "$FLIGHT_MESSAGES"
fi

fail=0

check_session() {
  local label="$1"
  local session_id="$2"
  local other_session_id="$3"
  local messages_file="$4"
  local topic_word="$5"

  if [ ! -s "$messages_file" ]; then
    echo "FAIL: no messages found for $label session"
    fail=1
    return
  fi

  local wrong_session
  wrong_session=$(jq -s --arg sid "$session_id" '[.[] | select(.session_id != $sid)] | length' "$messages_file")
  if [ "$wrong_session" -gt 0 ]; then
    echo "FAIL: $label queue contains $wrong_session message(s) NOT belonging to it"
    fail=1
  fi

  local has_other
  has_other=$(jq -s --arg sid "$other_session_id" '[.[] | select(.session_id == $sid)] | length' "$messages_file")
  if [ "$has_other" -gt 0 ]; then
    echo "FAIL: $label queue contains $has_other message(s) belonging to the OTHER session - cross-contamination"
    fail=1
  fi

  local steps_count
  steps_count=$(jq -s '[.[] | select(.kind == "steps")] | length' "$messages_file")
  if [ "$steps_count" -eq 0 ]; then
    echo "FAIL: $label session has no 'steps' messages at all"
    fail=1
  fi

  if ! jq -s -e --arg w "$topic_word" '[.[] | tostring | ascii_downcase | contains($w)] | any' "$messages_file" > /dev/null; then
    echo "FAIL: $label session's messages never mention '$topic_word' - may be testing the wrong content"
    fail=1
  fi

  echo "  $label: $(wc -l < "$messages_file" | tr -d ' ') message(s), own-session only, mentions '$topic_word'"
}

check_session "shopping" "$SHOPPING_SESSION" "$FLIGHT_SESSION" "$SHOPPING_MESSAGES" "amazon"
check_session "flight" "$FLIGHT_SESSION" "$SHOPPING_SESSION" "$FLIGHT_MESSAGES" "flight"

if [ "$fail" -ne 0 ]; then
  echo "--- shopping messages, for debugging ---"
  jq . "$SHOPPING_MESSAGES" 2>/dev/null || cat "$SHOPPING_MESSAGES"
  echo "--- flight messages, for debugging ---"
  jq . "$FLIGHT_MESSAGES" 2>/dev/null || cat "$FLIGHT_MESSAGES"
  exit 1
fi

echo "PASS: shopping session (worker.$SHOPPING_QUEUE) and flight session (worker.$FLIGHT_QUEUE) stayed fully isolated, each landing only its own messages"
