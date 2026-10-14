#!/usr/bin/env bash
# Integration test for the UserPromptSubmit hook.
#
# Registers the real `recall` binary as the UserPromptSubmit hook (no matcher - that
# event doesn't support one) via an isolated --settings file (never touches this repo's
# own .claude config, so it can't affect any other session), then runs two real
# `claude -p` turns and reads the result straight out of Claude Code's own
# --include-hook-events stream (not a side-channel log file):
#
#   1. a prompt that mentions a website and a password-looking string - the secret must
#      never appear in anything recall itself emitted, and recall must report redacting
#      it; and because the prompt is clearly browser-related, recall's
#      browser_predicate must let find_hint run, so the forced GROOVE_TEST_HINT must
#      come back verbatim as hookSpecificOutput.additionalContext
#   2. a prompt with nothing browser-related in it at all - browser_predicate must
#      suppress the hint lookup, so even with GROOVE_TEST_HINT set, the response must
#      come back empty ({}). This is the real test of the predicate itself: if it
#      wrongly said "yes" here, the hint would still appear, since find_hint doesn't
#      otherwise care what the prompt says.
#
# find_hint has no real memory store yet (always returns None) - see recall/hint.rs -
# so GROOVE_TEST_HINT forces a deterministic hint for this run only, to prove the
# delivery plumbing (and the predicate gating it) work independent of whether real
# memory exists.
#
# NOTE: every JSON-bearing variable below is piped with `printf '%s\n'`, never `echo` -
# see pretooluse_recall_test.sh for why `echo` silently corrupts this.
#
# Requires: an authenticated `claude` CLI and jq.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RECALL_BIN="$REPO_ROOT/target/debug/recall"
SECRET="sk-testsecretvalueabcdefghijklmnop"
TEST_HINT="groove-test-hint-$(date +%s)"

echo "==> building recall"
(cd "$REPO_ROOT" && cargo build --quiet --bin recall)

TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT

cat > "$TEST_DIR/settings.json" <<EOF
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          { "type": "command", "command": "$RECALL_BIN" }
        ]
      }
    ]
  }
}
EOF

run_prompt() {
  local prompt="$1"
  local out_file="$2"
  (
    cd "$TEST_DIR"
    GROOVE_TEST_HINT="$TEST_HINT" claude -p \
      "$prompt" \
      --settings "$TEST_DIR/settings.json" \
      --dangerously-skip-permissions \
      --output-format stream-json \
      --include-hook-events \
      --verbose \
      < /dev/null \
      > "$out_file"
  )
}

fail=0

# ===== 1. Browser-related prompt: redaction + hint delivery =====

echo "==> running claude -p with a browser-related prompt containing a secret"
BROWSER_OUT="$TEST_DIR/browser.jsonl"
run_prompt "Check out this website at https://example.com and log in - the password is ${SECRET}." "$BROWSER_OUT"

BROWSER_RESPONSE=$(jq -c 'select(.subtype == "hook_response" and .hook_name == "UserPromptSubmit")' "$BROWSER_OUT" | head -n1)

if [ -z "$BROWSER_RESPONSE" ]; then
  echo "FAIL: no UserPromptSubmit hook_response found for the browser-related prompt"
  fail=1
else
  stdout_field=$(printf '%s\n' "$BROWSER_RESPONSE" | jq -r '.stdout // empty')
  stderr_field=$(printf '%s\n' "$BROWSER_RESPONSE" | jq -r '.stderr // empty')
  exit_code=$(printf '%s\n' "$BROWSER_RESPONSE" | jq -r '.exit_code')

  if printf '%s\n%s\n' "$stdout_field" "$stderr_field" | grep -qF "$SECRET"; then
    echo "FAIL: raw secret leaked into recall's own output"
    fail=1
  fi
  if ! printf '%s\n' "$stderr_field" | grep -q "redacted"; then
    echo "FAIL: recall did not report redacting the password"
    fail=1
  fi
  if [ "$exit_code" != "0" ]; then
    echo "FAIL: UserPromptSubmit hook exited non-zero ($exit_code)"
    fail=1
  fi

  returned_hint=$(printf '%s\n' "$stdout_field" | jq -r '.hookSpecificOutput.additionalContext // empty')
  if [ "$returned_hint" != "$TEST_HINT" ]; then
    echo "FAIL: hint did not round-trip for the browser-related prompt."
    echo "  expected: $TEST_HINT"
    echo "       got: $returned_hint"
    fail=1
  fi
fi

# ===== 2. Non-browser prompt: browser_predicate must suppress the hint =====

echo "==> running claude -p with an unrelated, non-browser prompt"
PLAIN_OUT="$TEST_DIR/plain.jsonl"
run_prompt "Explain how binary search works, in one sentence." "$PLAIN_OUT"

PLAIN_RESPONSE=$(jq -c 'select(.subtype == "hook_response" and .hook_name == "UserPromptSubmit")' "$PLAIN_OUT" | head -n1)

if [ -z "$PLAIN_RESPONSE" ]; then
  echo "FAIL: no UserPromptSubmit hook_response found for the non-browser prompt"
  fail=1
else
  plain_stdout=$(printf '%s\n' "$PLAIN_RESPONSE" | jq -r '.stdout // empty')
  plain_exit_code=$(printf '%s\n' "$PLAIN_RESPONSE" | jq -r '.exit_code')

  if [ "$plain_exit_code" != "0" ]; then
    echo "FAIL: UserPromptSubmit hook exited non-zero ($plain_exit_code) for the non-browser prompt"
    fail=1
  fi

  # GROOVE_TEST_HINT is set for this run too - if this is non-empty, browser_predicate
  # failed to gate the lookup, since find_hint itself doesn't look at prompt content.
  plain_hint=$(printf '%s\n' "$plain_stdout" | jq -r '.hookSpecificOutput.additionalContext // empty')
  if [ -n "$plain_hint" ]; then
    echo "FAIL: browser_predicate let the hint through for a prompt with nothing browser-related in it"
    echo "  got hint: $plain_hint"
    fail=1
  fi
fi

if [ "$fail" -ne 0 ]; then
  echo "--- browser-prompt hook_response, for debugging ---"
  printf '%s\n' "$BROWSER_RESPONSE" | jq . 2>/dev/null || true
  echo "--- plain-prompt hook_response, for debugging ---"
  printf '%s\n' "$PLAIN_RESPONSE" | jq . 2>/dev/null || true
  exit 1
fi

echo "PASS: password redacted and hint delivered on a browser prompt; hint correctly suppressed on an unrelated prompt"
