# Groove — agent setup

A memory system for Claude Code's browser use. Three Rust binaries (`capture`, `recall`,
plus `integrations/rabbitmq`'s setup tools) and one Python async worker (`learn`), talking
through Redis, Qdrant, and RabbitMQ. Full architecture: see `CLAUDE.md` and `README.md`'s
system-design diagram.

## Setup

```sh
docker-compose up -d                          # redis, rabbitmq, qdrant
cargo run --bin rabbitmq-setup                # declares exchange/queues (once per fresh rabbitmq)
cargo build --release
cd learn && uv sync
```

Requires `OPENROUTER_API_KEY` in `.env` for `learn`'s LLM calls (HMT segmentation, AWM
induction). Everything else (`REDIS_URL`, `QDRANT_URL`, `QDRANT_GRPC_URL`, `RABBITMQ_URL`)
has a working localhost default.

Start the worker: `cd learn && ./.venv/bin/learn`

## Code standards

- Rust: no new crate dependency without checking it actually compiles against the
  workspace's resolved versions first (`ort`/`fastembed`/`qdrant-client` have had real
  cross-version API breaks in this project) — verified live, not assumed.
- Every similarity-matching function (intent, workflow, fact, stage-pick) needs a floor
  *and* margin check, not a floor alone — a lone high score with no real runner-up to
  compare against has repeatedly proven exploitable in this codebase. See `CLAUDE.md`'s
  "Conventions worth knowing" for the specific cases already found.
- No new matching/threshold logic ships without a benchmark against real captured data
  (`benchmarks/`) first — synthetic/invented test phrases have repeatedly given false
  confidence here.

## Test requirements

- `cargo test` before any commit touching Rust — no external services required.
- For anything touching `recall`'s hint logic: run the relevant `evals/*.sh` live
  integration test (needs Redis/RabbitMQ/Qdrant up) or manually verify with real seeded
  Qdrant/Redis data, the same way every piece in this codebase was verified during
  development — not just a unit test in isolation.
- For anything touching a matching threshold: run or extend the matching `benchmarks/*.py`
  script for that mechanism before relying on a new floor/margin number.

## Development workflow

- `recall` and `capture` are never wired into this repo's own `.claude/` hooks globally —
  tests and demos use an isolated `--settings` file passed to `claude -p`, so running them
  never affects an unrelated session.
- Prefer extending an existing matcher's pattern (floor+margin via
  `qdrant_store::clears_floor_and_margin`) over inventing a new one.

## Forbidden

- Do not fix `learn/src/learn/key_match.py`'s known First-Name/Last-Name-style merge bug
  (see `CLAUDE.md`) without discussing scope first — it's shared by `facts.py`,
  `field_pairs.py`, AWM, and HMT's intent resolution, so a fix needs its own benchmark
  pass across all of them, not a local patch.
- Do not add LLM calls to `recall` — it runs inside a synchronous Claude Code hook and
  must stay fast; all matching there uses local embeddings or plain string comparison.
