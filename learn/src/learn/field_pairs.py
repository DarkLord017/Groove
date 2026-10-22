"""Pairs a type/key step's value with the field label of whatever was clicked right
before it - e.g. click "textbox \"Destination\"" then type "Roorkee" becomes
{"key": "Destination", "value": "Roorkee"}.
"""
import json
import os

import redis

from learn.errors import TransientFailure

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
TTL_SECONDS = 60 * 60

_CLICK_ACTIONS = {"left_click", "right_click", "double_click", "triple_click", "left_click_drag"}
_TYPE_ACTIONS = {"type", "key"}

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)


def _last_click_key(session_id: str, tab_id) -> str:
    return f"fields:lastclick:{session_id}:{tab_id}"


def _pairs_key(session_id: str) -> str:
    return f"fields:pairs:{session_id}"


def observe_step(session_id: str, payload: dict) -> None:
    """Looks at one steps payload: remembers the field label on a click, pairs it with
    the value on the next type/key for the same tab. No-op for anything else.
    """
    computer = payload.get("tool_input", {}).get("Computer")
    if not computer:
        return

    action = computer.get("action")
    tab_id = computer.get("tabId")
    if tab_id is None:
        return

    try:
        if action in _CLICK_ACTIONS:
            label = computer.get("ref")
            if label:
                _client.set(_last_click_key(session_id, tab_id), label, ex=TTL_SECONDS)
        elif action in _TYPE_ACTIONS and payload.get("tool_response") is not None:
            value = computer.get("text")
            if not value:
                return
            label = _client.get(_last_click_key(session_id, tab_id))
            if label:
                _client.rpush(_pairs_key(session_id), json.dumps({"key": label, "value": value}))
                _client.expire(_pairs_key(session_id), TTL_SECONDS)
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs observe_step failed: {e}") from e


def peek(session_id: str) -> list[dict]:
    try:
        raw = _client.lrange(_pairs_key(session_id), 0, -1)
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs peek failed: {e}") from e
    return [json.loads(item) for item in raw]


def clear(session_id: str) -> None:
    try:
        _client.delete(_pairs_key(session_id))
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs clear failed: {e}") from e
