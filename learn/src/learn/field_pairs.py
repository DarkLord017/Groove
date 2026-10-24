"""Pairs a type/key step's value with the field label of whatever was clicked right
before it - e.g. click "textbox \"Destination\"" then type "Roorkee" becomes
{"key": "Destination", "value": "Roorkee"}.

form_input is a second, simpler path to the same pair: it's claude-in-chrome's
one-shot "set this field's value" tool, so a single call already carries both the
field's resolved label (ref) and the value - no click-then-type pairing needed.
"""
import json
import os
import re

import redis

from learn import key_match
from learn.errors import TransientFailure

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
TTL_SECONDS = 60 * 60

_CLICK_ACTIONS = {"left_click", "right_click", "double_click", "triple_click", "left_click_drag"}
_TYPE_ACTIONS = {"type", "key"}
_CHECKBOX_ROLE = "checkbox"

# Custom dropdowns (not a native <select>, which would come through as form_input)
# pick a value via a SECOND click, on an option labeled "Select option: <value>" -
# a structured, site-independent pattern, unlike a generic role such as "button" or
# "presentation" that could mean anything.
_SELECT_OPTION_PATTERN = re.compile(r'^\S+\s+"Select option:\s*(.+)"$', re.IGNORECASE)

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)


def _last_click_key(session_id: str, tab_id) -> str:
    return f"fields:lastclick:{session_id}:{tab_id}"


def _pairs_key(session_id: str) -> str:
    return f"fields:pairs:{session_id}"


def _is_checkbox_label(label: str) -> bool:
    # Resolved labels are "role \"name\"" (see capture/elements.rs's describe()) -
    # the role is always the first word.
    return label.split(" ", 1)[0].lower() == _CHECKBOX_ROLE


def _select_option_value(label: str) -> str | None:
    match = _SELECT_OPTION_PATTERN.match(label)
    return match.group(1) if match else None


def _as_text(value) -> str:
    # form_input's value can be a bool for a checkbox - "true"/"false" reads like
    # leftover code, "yes"/"no" reads like the actual fact being recorded.
    if isinstance(value, bool):
        return "yes" if value else "no"
    return str(value)


def _save_pair(session_id: str, label: str, value: str) -> None:
    existing_keys = list(dict.fromkeys(p["key"] for p in peek(session_id)))
    key = key_match.find_match(label, existing_keys) or label
    _client.rpush(_pairs_key(session_id), json.dumps({"key": key, "value": value.lower()}))
    _client.expire(_pairs_key(session_id), TTL_SECONDS)


def _flush_pending_click(session_id: str, tab_id) -> None:
    """A click that set lastclick (waiting for a type) but never got one before being
    overwritten/abandoned IS itself the whole action - e.g. clicking a "Search" or
    "Confirm" button with no value of its own. Save it as a bare "clicked" pair
    instead of silently losing it, then clear the slot so a later type doesn't wrongly
    re-attach to it.
    """
    key = _last_click_key(session_id, tab_id)
    pending = _client.get(key)
    if pending:
        # Delete before saving: _save_pair calls peek(), which flushes pending
        # clicks too - deleting first stops that from finding this same key still
        # set and recursing back into this function forever.
        _client.delete(key)
        _save_pair(session_id, pending, "clicked")


def _is_error_response(tool_response) -> bool:
    """claude-in-chrome wraps a failed tool call's message in literal <error> tags
    (verified live: a bad ref, a bad tabId, a broken page all come back this way) -
    checking for it stops a failed click/type from being recorded as if it worked.
    Note: navigate does NOT follow this pattern - it reports success even when the
    destination fails to load, so this check alone won't catch a bad navigate.
    """
    if isinstance(tool_response, str):
        return "<error>" in tool_response
    if isinstance(tool_response, list):
        return any(
            isinstance(block, dict) and isinstance(block.get("text"), str) and "<error>" in block["text"]
            for block in tool_response
        )
    return False


def observe_step(session_id: str, payload: dict) -> None:
    """Looks at one steps payload: remembers the field label on a click, pairs it with
    the value on the next type/key for the same tab (Computer), or takes the label and
    value straight from a single form_input call. No-op for anything else.
    """
    tool_response = payload.get("tool_response")
    if tool_response is None or _is_error_response(tool_response):
        return

    tool_input = payload.get("tool_input", {})

    form_input = tool_input.get("FormInput")
    if form_input:
        label = form_input.get("ref")
        value = form_input.get("value")
        if not label or value is None:
            return
        try:
            _save_pair(session_id, label, _as_text(value))
        except redis.RedisError as e:
            raise TransientFailure(f"field_pairs observe_step failed: {e}") from e
        return

    computer = tool_input.get("Computer")
    if not computer:
        return

    action = computer.get("action")
    tab_id = computer.get("tabId")
    if tab_id is None:
        return

    try:
        if action in _CLICK_ACTIONS:
            label = computer.get("ref")
            if not label:
                pass
            else:
                option_value = _select_option_value(label)
                if option_value is not None:
                    # This click picks a value for whatever dropdown was opened by
                    # the PREVIOUS click - it isn't itself a field, so it must not
                    # overwrite lastclick the way an ordinary click would.
                    lastclick_key = _last_click_key(session_id, tab_id)
                    field_label = _client.get(lastclick_key)
                    if field_label:
                        # Delete before saving - _save_pair's internal peek() flushes
                        # pending clicks too, and would otherwise find this same key
                        # still set and wrongly re-save it as an abandoned click.
                        _client.delete(lastclick_key)
                        _save_pair(session_id, field_label, option_value)
                else:
                    # Either branch below is about to claim/overwrite lastclick -
                    # flush whatever was waiting there first, or it's lost for good.
                    _flush_pending_click(session_id, tab_id)
                    if _is_checkbox_label(label):
                        # A checkbox click is the whole action - there's no type
                        # action coming after it to pair with.
                        _save_pair(session_id, label, "yes")
                    else:
                        _client.set(_last_click_key(session_id, tab_id), label, ex=TTL_SECONDS)
        elif action in _TYPE_ACTIONS:
            value = computer.get("text")
            if not value:
                return
            key = _last_click_key(session_id, tab_id)
            label = _client.get(key)
            if label:
                _client.delete(key)  # delete before saving - see the click branch above
                _save_pair(session_id, label, value)
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs observe_step failed: {e}") from e


def _flush_all_pending_clicks(session_id: str) -> None:
    """Catches the one case per-click flushing can't: a click left pending with no
    further click/type ever following it in the whole run (e.g. the run's last
    action is clicking a result to confirm it). Scans every tab this session touched.
    """
    pattern = f"fields:lastclick:{session_id}:*"
    for key in _client.scan_iter(match=pattern):
        pending = _client.get(key)
        _client.delete(key)  # delete before saving - see _flush_pending_click
        if pending:
            _save_pair(session_id, pending, "clicked")


def peek(session_id: str) -> list[dict]:
    try:
        _flush_all_pending_clicks(session_id)
        raw = _client.lrange(_pairs_key(session_id), 0, -1)
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs peek failed: {e}") from e
    return [json.loads(item) for item in raw]


def clear(session_id: str) -> None:
    try:
        _client.delete(_pairs_key(session_id))
    except redis.RedisError as e:
        raise TransientFailure(f"field_pairs clear failed: {e}") from e
