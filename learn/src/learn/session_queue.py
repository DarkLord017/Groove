import os

import redis

from learn.errors import TransientFailure

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
TTL_SECONDS = 60 * 60

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)


def _key(session_id: str) -> str:
    return f"queue:{session_id}"


def remember(session_id: str, queue_num) -> None:
    if queue_num is None:
        return
    try:
        _client.set(_key(session_id), queue_num, ex=TTL_SECONDS)
    except redis.RedisError as e:
        raise TransientFailure(f"session_queue remember failed: {e}") from e


def get(session_id: str) -> int | None:
    try:
        value = _client.get(_key(session_id))
    except redis.RedisError as e:
        raise TransientFailure(f"session_queue get failed: {e}") from e
    return int(value) if value is not None else None
