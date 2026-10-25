# Groove

A memory system for Claude Code's browser use (`claude-in-chrome`). It watches what the
agent does in the browser, learns reusable workflows/stages/facts from successful runs,
and hints them back on future runs. See `README.md` for the system-design diagram.

## Architecture

Three Rust binaries + one Python worker, talking through Redis/Qdrant/RabbitMQ:

- **`capture`** (Rust) — PostToolUse/Stop/SessionEnd hooks. Caches page elements (ref →
  description) in Redis, resolves refs before publishing, and publishes "steps"/"prompt"/
  "turn.complete" events to RabbitMQ for `learn` to consume. Write-side observer, never
  blocks on external calls beyond Redis/RabbitMQ.
- **`recall`** (Rust) — PreToolUse/UserPromptSubmit/PostToolUse/Stop hooks. Reads directly
  from Qdrant (via `integrations/qdrant`) and Redis to produce hints as `additionalContext`.
  Read-only: never writes new intents/stages/workflows, only what `learn` already wrote.
  Runs a local embedding model (fastembed MiniLM) — no LLM calls, hooks must be fast.
- **`learn`** (Python, `learn/`) — async RabbitMQ consumer (`learn/src/learn/consumer.py`,
  entry point `learn`). Does the LLM-backed work: segments runs into HMT stages
  (`hmt/stage_llm.py`), induces AWM workflows (`awm/workflow.py`), judges run success
  (`jev/`), extracts personal facts (`facts.py`). This is where all writes to Qdrant happen.
- **`shared`** (Rust) — hook request/response types, `ChromeToolInput` tool-call parsing,
  RabbitMQ message shaping, privacy redaction. Used by `capture` and `recall`.
- **`integrations/{redis,rabbitmq,qdrant}`** (Rust) — thin clients. `qdrant` additionally
  owns the local embedding model and all matching logic (floor+margin thresholds).

## Storage

- **Redis** — session/tab-scoped, TTL'd: page element cache, cached intent/domain per
  tab, done-stages set per run, recent-prompt window. Cleared on `Stop`/`SessionEnd`.
- **Qdrant** — durable, the actual "memory": `site_stages` (HMT), `site_intents` (HMT),
  `site_workflows` (AWM), `user_facts`, `user_routines`. Written only by `learn`, read by
  both `learn` (to resolve/dedup) and `recall` (to hint).
- **RabbitMQ** — event bus from `capture`/`recall` to `learn`. 32 worker queues, topic
  exchange `groove.events`, DLQ on permanent failures. Must run `rabbitmq-setup` once
  before anything publishes (see below).

## Build & run

```sh
docker-compose up -d                          # redis, rabbitmq, qdrant
cargo run --bin rabbitmq-setup                # declares exchange/queues (once per fresh rabbitmq)
cargo build --release                         # capture, recall, + the rabbitmq-* tools
cd learn && uv sync && ./.venv/bin/learn      # starts the async consumer
```

`recall`/`capture` are registered as Claude Code hooks via a `settings.json` (see
`evals/*.sh` for the exact hook shape) — nothing in this repo's own `.claude/` wires them
up globally, by design, so running the test suite or demos never affects other sessions.

Env vars (all optional, sensible localhost defaults): `REDIS_URL`, `QDRANT_URL` (REST,
used by Python), `QDRANT_GRPC_URL` (used by Rust), `RABBITMQ_URL`, `OPENROUTER_API_KEY`
(required for `learn`'s LLM calls — segmentation and AWM induction), `FASTEMBED_CACHE_DIR`
(defaults to `~/.cache/groove/fastembed`, not cwd-relative).

## Testing

- `cargo test` — unit tests, no external services needed.
- `evals/*.sh` — live integration tests: build the real binaries, run `claude -p --chrome`
  against a local test page with hooks registered via an isolated `--settings` file, assert
  on the real hook_response stream. Needs Redis/RabbitMQ/Qdrant running.
- `benchmarks/*.py` — matching-accuracy checks against real captured data (never
  synthetic/invented examples), e.g. `stage_key_overlap_benchmark.py`,
  `awm_intent_match_benchmark.py`, `page_condition_match_benchmark.py`. Run before trusting
  any new floor/margin threshold or matching approach.

## Conventions worth knowing

- Every matching mechanism (intent, workflow, fact, stage-pick) uses a floor *and* margin
  check — a single score threshold alone has repeatedly proven exploitable by
  look-alike-but-wrong candidates. With no real runner-up to compare against, floor alone
  decides (an accepted, deliberate trade-off, not an oversight).
- HMT's stage *selection* (`select_stage`/`candidate_stages`) is deliberately cross-site
  (no domain filter) for generalization; HMT's stage *storage identity* and AWM are
  domain-scoped. This split is intentional, not inconsistent — see `site_tree.py`'s
  docstrings.
- Page-text-vs-condition matching uses word containment, not embeddings — embedding
  similarity between a raw page dump and a short condition sentence was tested and found
  unreliable (see `benchmarks/page_condition_match_benchmark.py`).
- Known bug, not yet fixed: `learn/src/learn/key_match.py`'s `find_match` can merge two
  genuinely different short field-key labels (e.g. "First Name" vs "Last Name") into one
  stored fact when they share enough surface text. Affects `facts.py`, `field_pairs.py`,
  and anywhere else `find_match` resolves a key against existing ones.
