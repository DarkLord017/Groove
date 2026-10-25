import os
import time

import httpx
from pydantic import BaseModel, ValidationError

from learn.errors import TransientFailure

OPENROUTER_BASE_URL = os.environ.get("OPENROUTER_BASE_URL", "https://openrouter.ai")
OPENROUTER_API_KEY = os.environ.get("OPENROUTER_API_KEY")
SEGMENTER_MODEL = os.environ.get("SEGMENTER_MODEL", "qwen/qwen-2.5-7b-instruct")
SEGMENTER_TIMEOUT_SECONDS = 30.0
SEGMENTER_MAX_ATTEMPTS = int(os.environ.get("SEGMENTER_MAX_ATTEMPTS", "3"))
SEGMENTER_BACKOFF_BASE_SECONDS = float(os.environ.get("SEGMENTER_BACKOFF_BASE_SECONDS", "1"))
SEGMENTER_MAX_TOKENS = 1000  # without an explicit cap, some models default to requesting tens of
# thousands of tokens, which can trip this account's credit limit (hit live, twice now, as a 402) -
# a handful of stage names/conditions never needs anywhere near that.


class Stage(BaseModel):
    name: str
    pre_condition: str
    post_condition: str
    pair_indices: list[int]


class Segmentation(BaseModel):
    stages: list[Stage]


_SEGMENTATION_SCHEMA = Segmentation.model_json_schema()

# The output shape is enforced via response_format (a Pydantic-generated JSON
# schema, same approach as awm/workflow.py), not described in prose here - one
# source of truth instead of two that could drift. Coverage/contiguity of
# pair_indices still needs its own check afterward (see _covers_every_index) -
# that's a cross-field constraint plain JSON schema can't express.
SYSTEM_PROMPT = """You segment a web agent's recorded actions into reusable stages of a task.

You will be given the user's overall task and an ordered list of indexed {key, value} \
actions the agent took in ONE run. Group the actions into contiguous stages - a stage \
is a short run of actions that together accomplish one sub-goal (e.g. "entering search \
criteria", "applying a filter", "picking a result", "confirming the choice").

For each stage, write a name (a short, reusable name for this sub-goal, 2-5 words - \
generic enough that the SAME kind of sub-goal on a different site, or a different run \
of this task, would get the same name - not a copy of the field labels themselves), a \
pre_condition (one short sentence describing what should be true/visible right BEFORE \
this stage starts), a post_condition (one short sentence describing what should be \
true/visible right AFTER this stage finishes), and pair_indices (the action indices, \
0-based, that belong to this stage, in order - every index from the input must appear \
in exactly one stage, contiguous and in increasing order across stages).

Rules:
- Only state what's directly supported by the given actions or the task description. \
Do not invent destinations, dates, prices, or steps not implied by the actions - if the \
task description mentions something this run's actions don't show, leave it out.
- Do not treat placeholder text or example values as real user selections.
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
                "max_tokens": SEGMENTER_MAX_TOKENS,
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": user_content},
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {"name": "segmentation", "strict": True, "schema": _SEGMENTATION_SCHEMA},
                },
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


def _covers_every_index(stages: list[Stage], pair_count: int) -> bool:
    """Checks every index 0..pair_count-1 appears exactly once, contiguous and in
    order across stages - matches what the prompt asked for. This is a cross-field
    constraint (depends on the caller's pair_count) that plain JSON schema can't
    express, so it's still checked here even though the shape itself is now
    schema-enforced.
    """
    seen: list[int] = []
    for stage in stages:
        if not stage.pair_indices:
            return False
        seen.extend(stage.pair_indices)
    return seen == list(range(pair_count))


def segment_pairs(intent: str, pairs: list[dict]) -> list[Stage] | None:
    """Segments pairs into stages with real semantic names and conditions, via one
    OpenRouter chat-completion call. The response shape is enforced by OpenRouter's
    structured-output mode against a Pydantic schema (verified live - see
    awm/workflow.py for the same approach), not just asked for in prose. Returns
    None on any failure (missing key, malformed JSON, schema violation, or
    incomplete/non-contiguous index coverage) - callers should skip recording this
    run's stages rather than guess at a broken structure.
    """
    if not pairs:
        return []
    if not OPENROUTER_API_KEY:
        raise RuntimeError("OPENROUTER_API_KEY is not set - cannot call the segmenter")

    data = _call_with_retry(intent, pairs)
    try:
        content = data["choices"][0]["message"]["content"]
        segmentation = Segmentation.model_validate_json(content)
    except (KeyError, IndexError, TypeError, ValidationError):
        return None

    if not _covers_every_index(segmentation.stages, len(pairs)):
        return None
    return segmentation.stages
