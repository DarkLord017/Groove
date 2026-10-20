
import os

import redis

from learn.errors import TransientFailure

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
TTL_SECONDS = 60 * 60  # generous upper bound on how long a single run should take

# Bounds on what gets sent to JEV - a long run shouldn't be able to build an unbounded
# request (JEV caps request bodies at 32 KiB). Keeps the most recent steps, since
# they're the most relevant to judging the run's actual outcome.
JEV_MAX_STEPS = 20
JEV_MAX_STEP_LENGTH = 500

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)


def _objective_key(session_id: str) -> str:
    return f"jev:objective:{session_id}"


def _work_key(session_id: str) -> str:
    return f"jev:work:{session_id}"


def set_objective(session_id: str, prompt: str) -> None:
    try:
        _client.set(_objective_key(session_id), prompt, ex=TTL_SECONDS)
    except redis.RedisError as e:
        raise TransientFailure(f"jev_session set_objective failed: {e}") from e


def append_work(session_id: str, line: str) -> None:
    key = _work_key(session_id)
    try:
        _client.rpush(key, line)
        _client.ltrim(key, -JEV_MAX_STEPS, -1)  # keep only the most recent N, oldest-first order preserved
        _client.expire(key, TTL_SECONDS)
    except redis.RedisError as e:
        raise TransientFailure(f"jev_session append_work failed: {e}") from e


def summarize_step(label: str, payload: dict) -> str:
    """Turns one steps/page.snapshot payload into a single readable line for
    completed_work. tool_response is the useful part when present (what actually
    happened); otherwise falls back to the raw payload. Truncated to JEV_MAX_STEP_LENGTH
    so one chatty tool response can't dominate the request on its own.
    """
    tool_response = payload.get("tool_response")
    line = f"{label}: {tool_response}" if tool_response is not None else f"{label}: {payload}"
    if len(line) > JEV_MAX_STEP_LENGTH:
        line = line[:JEV_MAX_STEP_LENGTH] + "...(truncated)"
    return line


def peek(session_id: str) -> tuple[str, list[str]]:
    """Returns the session's accumulated (objective, completed_work) without clearing
    it. Non-destructive so a failed JEV call can be retried (including via a message
    requeue) without having already lost the state it needs.
    """
    try:
        objective = _client.get(_objective_key(session_id)) or ""
        completed_work = _client.lrange(_work_key(session_id), 0, -1)
    except redis.RedisError as e:
        raise TransientFailure(f"jev_session peek failed: {e}") from e
    return objective, completed_work


def clear(session_id: str) -> None:
    """Drops a session's accumulated state. Called once the JEV call has a final
    outcome - success, or a failure that won't be retried - so state doesn't leak
    forever for sessions that are done with.
    """
    try:
        _client.delete(_objective_key(session_id), _work_key(session_id))
    except redis.RedisError as e:
        raise TransientFailure(f"jev_session clear failed: {e}") from e
