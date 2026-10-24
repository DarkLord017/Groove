"""Segments a run's field_pairs into Stage occurrences via one real LLM call over
OpenRouter's normal chat-completions endpoint - NOT jev's decisions API, which only
answers bounded choice/score/noul questions and can't generate free text. This is
HMT's actual "Subgoal Segmentation" step: an LLM partitions the trajectory into
contiguous segments, each with a semantic name and real pre/post-condition
sentences - not a fixed category, and not just "whatever the neighbor stage is
called" (the gap in the first version of this: see site_tree.py's history).

Verified live (real goindigo.in page, same rules) that a mid-size instruct model
over OpenRouter describes state accurately without hallucinating from the task text,
where every small local model we tried (T5/BART/FLAN-T5/Qwen-0.5B/1.5B) failed the
same way: inventing details ("payment details", a fake destination) that were in the
task description but not actually present. This module applies that same approach to
segmenting+describing a run's field_pairs.
"""
import json
import os
import time

import httpx

from learn.errors import TransientFailure

OPENROUTER_BASE_URL = os.environ.get("OPENROUTER_BASE_URL", "https://openrouter.ai")
OPENROUTER_API_KEY = os.environ.get("OPENROUTER_API_KEY")
SEGMENTER_MODEL = os.environ.get("SEGMENTER_MODEL", "qwen/qwen-2.5-7b-instruct")
SEGMENTER_TIMEOUT_SECONDS = 30.0
SEGMENTER_MAX_ATTEMPTS = int(os.environ.get("SEGMENTER_MAX_ATTEMPTS", "3"))
SEGMENTER_BACKOFF_BASE_SECONDS = float(os.environ.get("SEGMENTER_BACKOFF_BASE_SECONDS", "1"))

SYSTEM_PROMPT = """You segment a web agent's recorded actions into reusable stages of a task.

You will be given the user's overall task and an ordered list of indexed {key, value} \
actions the agent took in ONE run. Group the actions into contiguous stages - a stage \
is a short run of actions that together accomplish one sub-goal (e.g. "entering search \
criteria", "applying a filter", "picking a result", "confirming the choice").

For each stage, write:
- "name": a short, reusable name for this sub-goal (2-5 words) - generic enough that \
the SAME kind of sub-goal on a different site, or a different run of this task, would \
get the same name. Not a copy of the field labels themselves.
- "pre_condition": one short sentence describing what should be true/visible right \
BEFORE this stage starts.
- "post_condition": one short sentence describing what should be true/visible right \
AFTER this stage finishes.
- "pair_indices": the action indices (0-based integers) that belong to this stage, in \
order. Every index from the input must appear in exactly one stage, contiguous and in \
increasing order across stages (stage 1 covers indices 0..k, stage 2 covers k+1..m, etc).

Rules:
- Only state what's directly supported by the given actions or the task description. \
Do not invent destinations, dates, prices, or steps not implied by the actions - if the \
task description mentions something this run's actions don't show, leave it out.
- Do not treat placeholder text or example values as real user selections.
- Return ONLY a JSON object of the exact shape {"stages": [...]} - no explanation, no \
markdown fences, no text outside the JSON.
"""


def _post_once(intent: str, pairs: list[dict]) -> dict:
    numbered = "\n".join(f"{i}: key={p['key']!r}, value={p['value']!r}" for i, p in enumerate(pairs))
    user_content = f'The user\'s task: "{intent}"\n\nActions in this run, in order:\n{numbered}'

    try:
        response = httpx.post(
            f"{OPENROUTER_BASE_URL}/api/v1/chat/completions",
            headers={"Authorization": f"Bearer {OPENROUTER_API_KEY}"},
            json={
                "model": SEGMENTER_MODEL,
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": user_content},
                ],
                "response_format": {"type": "json_object"},
            },
            timeout=SEGMENTER_TIMEOUT_SECONDS,
        )
    except httpx.TransportError as e:
        raise TransientFailure(f"stage_llm request failed: {e}") from e

    if response.status_code >= 500:
        raise TransientFailure(f"stage_llm returned {response.status_code}: {response.text}")
    response.raise_for_status()  # a 4xx (e.g. the credit-limit 402 we hit testing) won't be fixed by retrying
    return response.json()


def _call_with_retry(intent: str, pairs: list[dict]) -> dict:
    last_error: TransientFailure | None = None
    for attempt in range(1, SEGMENTER_MAX_ATTEMPTS + 1):
        try:
            return _post_once(intent, pairs)
        except TransientFailure as e:
            last_error = e
            if attempt < SEGMENTER_MAX_ATTEMPTS:
                time.sleep(SEGMENTER_BACKOFF_BASE_SECONDS * (2 ** (attempt - 1)))
    raise TransientFailure(f"stage_llm failed after {SEGMENTER_MAX_ATTEMPTS} attempts: {last_error}") from last_error


def _validate(stages: list, pair_count: int) -> bool:
    """Checks every index 0..pair_count-1 appears exactly once, contiguous and in
    order across stages - matches the schema we asked for. Malformed output is
    treated as a failure (skip the run) rather than guessed at.
    """
    seen: list[int] = []
    for stage in stages:
        if not isinstance(stage, dict):
            return False
        if not all(isinstance(stage.get(k), str) and stage[k] for k in ("name", "pre_condition", "post_condition")):
            return False
        indices = stage.get("pair_indices")
        if not isinstance(indices, list) or not indices:
            return False
        seen.extend(indices)
    return seen == list(range(pair_count))


def segment_pairs(intent: str, pairs: list[dict]) -> list[dict] | None:
    """Segments pairs into stages with real semantic names and conditions, via one
    OpenRouter chat-completion call. Returns None on any failure (missing key, bad
    JSON, malformed/incomplete coverage) - callers should skip recording this run's
    stages rather than guess at a broken structure.
    """
    if not pairs:
        return []
    if not OPENROUTER_API_KEY:
        raise RuntimeError("OPENROUTER_API_KEY is not set - cannot call the segmenter")

    data = _call_with_retry(intent, pairs)
    try:
        content = data["choices"][0]["message"]["content"]
        parsed = json.loads(content)
        stages = parsed["stages"]
    except (KeyError, IndexError, TypeError, json.JSONDecodeError):
        return None

    if not isinstance(stages, list) or not _validate(stages, len(pairs)):
        return None
    return stages
