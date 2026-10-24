import os
import time

import httpx

from learn.errors import TransientFailure

OPENROUTER_BASE_URL = os.environ.get("OPENROUTER_BASE_URL", "https://openrouter.ai")
OPENROUTER_API_KEY = os.environ.get("OPENROUTER_API_KEY")
JEV_MODEL = "typesafe/jev-1.13"  # pinned version, not a -latest alias
JEV_TIMEOUT_SECONDS = 10.0
JEV_MAX_ATTEMPTS = int(os.environ.get("JEV_MAX_ATTEMPTS", "4"))
JEV_BACKOFF_BASE_SECONDS = float(os.environ.get("JEV_BACKOFF_BASE_SECONDS", "1"))
JEV_SUCCESS_THRESHOLD = float(os.environ.get("JEV_SUCCESS_THRESHOLD", "0.5"))


def _post_once(state: dict, questions: dict) -> dict:
    try:
        response = httpx.post(
            f"{OPENROUTER_BASE_URL}/api/alpha/decisions",
            headers={"Authorization": f"Bearer {OPENROUTER_API_KEY}"},
            json={"model": JEV_MODEL, "state": state, "questions": questions},
            timeout=JEV_TIMEOUT_SECONDS,
        )
    except httpx.TransportError as e:
        raise TransientFailure(f"JEV request failed: {e}") from e

    if response.status_code >= 500:
        raise TransientFailure(f"JEV returned {response.status_code}: {response.text}")
    response.raise_for_status()  # a 4xx is our own bad request - retrying won't help
    return response.json()


def _call_with_retry(state: dict, questions: dict) -> dict:
    """Shared retry loop: exponential backoff on transient failures (network errors,
    5xx). A 4xx (malformed request) is raised immediately by _post_once - retrying it
    won't help.
    """
    last_error: TransientFailure | None = None
    for attempt in range(1, JEV_MAX_ATTEMPTS + 1):
        try:
            return _post_once(state, questions)
        except TransientFailure as e:
            last_error = e
            if attempt < JEV_MAX_ATTEMPTS:
                backoff = JEV_BACKOFF_BASE_SECONDS * (2 ** (attempt - 1))
                print(f"learn: jev call failed (attempt {attempt}/{JEV_MAX_ATTEMPTS}): {e} - retrying in {backoff}s")
                time.sleep(backoff)

    raise TransientFailure(f"JEV call failed after {JEV_MAX_ATTEMPTS} attempts: {last_error}") from last_error


def review_completion(objective: str, steps: list[str], agent_message: str) -> dict:
    """Asks Jev for a single "success" probability (0-1) given the objective, the
    actual recorded steps, and the agent's own final message. Steps are weighted over
    the agent's self-report in the instructions, since an agent's last message can
    overclaim relative to what it actually did.
    """
    if not OPENROUTER_API_KEY:
        raise RuntimeError("OPENROUTER_API_KEY is not set - cannot call JEV")

    state = {"objective": objective, "steps": steps, "agent_message": agent_message}
    questions = {
        "success": {
            "type": "noul",
            "instructions": (
                "Given the objective, the steps actually taken, and the agent's own "
                "final message, estimate the probability that the run successfully "
                "achieved the objective. Trust the steps over the agent's own claim "
                "if they disagree. A genuine best-effort answer under real-world "
                "ambiguity - naming a clear recommended option while also noting "
                "legitimate alternatives or caveats - still counts as success if it "
                "substantively addresses the objective. Do not penalize the agent for "
                "surfacing extra relevant options or honest caveats; only score it as "
                "a failure if the objective was not actually addressed at all."
            ),
        }
    }
    return _call_with_retry(state, questions)


def indicates_success(decision: dict) -> bool:
    """True if JEV's "success" answer clears JEV_SUCCESS_THRESHOLD. A missing or
    malformed answer counts as not successful - better to skip learning from a run
    than to learn from one JEV couldn't actually judge.
    """
    try:
        return decision["answers"]["success"]["noul"] >= JEV_SUCCESS_THRESHOLD
    except (KeyError, TypeError):
        return False


PAIR_CLASSIFICATION_CRITERIA = {
    "personal": (
        "a durable fact ABOUT WHO THE USER IS, unrelated to any particular task - "
        "true again in every future run no matter what they're doing (name, email, "
        "phone, home address, date of birth, stated preferences). "
        "Example: key='Your Name', value='Sambhav Jain' -> personal."
    ),
    "task": (
        "a value describing what the user is DOING in this run, where this SAME "
        "VALUE is plausibly going to come up again on a future, similar run - a "
        "destination, a recipient, a chosen plan/tier, a preferred time. The test is "
        "the value itself repeating, not just the kind of field repeating. "
        "Example: key='Destination', value='Delhi' -> task (if the user flies to "
        "Delhi often, this exact value recurs)."
    ),
    "not_needed": (
        "a value generated fresh per run that will essentially NEVER be the same "
        "value twice, even if the user repeats the same kind of task - order IDs, "
        "confirmation codes, CAPTCHA text, timestamps, one-off search query text, "
        "transaction references. Example: key='Order ID', value='A19283' -> "
        "not_needed (the next order gets a different ID even if the user orders "
        "again)."
    ),
}

PAIR_CLASSIFICATION_STEPS = (
    "Work through these in order and stop at the first one that applies: "
    "(1) Is this true about the user no matter what task they're doing? -> personal. "
    "(2) If not, is this exact VALUE likely to be the same again next time the user "
    "does a similar task? -> task. "
    "(3) If the value is essentially unique/random and won't repeat even if the task "
    "repeats -> not_needed. "
    "Pick exactly one."
)


# No fixed vocabulary for stages (unlike PAIR_CLASSIFICATION_CRITERIA's personal/
# task/not_needed) - a stage's identity is decided later by semantic similarity to
# previously stored stages (site_tree._resolve_stage_label), the same way facts/
# tasks/intents already work. This only decides WHERE one stage ends and the next
# begins; it doesn't name anything. Jev sees the whole pairs sequence as context for
# every question, so a boundary call isn't just "compare these two pairs in
# isolation" - it can reason about the pair's role in the full run.
STAGE_BOUNDARY_CRITERIA = {
    "same_stage": "this pair continues the same sub-goal as the pair right before it.",
    "new_stage": "this pair starts a new, distinct sub-goal compared to the pair right before it.",
}


def classify_pairs(pairs: list[dict]) -> dict:
    """Asks Jev to classify each {key, value} pair as personal / task / not_needed,
    AND (same call, same state, extra questions - not a second request) where one
    stage ends and the next begins for site_tree's Stage segmentation. One call
    covers every pair instead of one call per pair, and bundling both question sets
    into this single call keeps a run at 2 total Jev calls (this + review_completion)
    instead of 3. Returns the raw decision dict - read it back with
    pair_classification/pair_boundary.
    """
    if not pairs:
        return {}
    if not OPENROUTER_API_KEY:
        raise RuntimeError("OPENROUTER_API_KEY is not set - cannot call JEV")

    state = {"pairs": pairs}
    questions = {
        f"pair_{i}": {
            "type": "choice",
            "criteria": PAIR_CLASSIFICATION_CRITERIA,
            "instructions": (
                f"Classify state.pairs[{i}] (key={pair['key']!r}, value={pair['value']!r}). "
                f"{PAIR_CLASSIFICATION_STEPS}"
            ),
        }
        for i, pair in enumerate(pairs)
    }
    for i in range(1, len(pairs)):
        questions[f"pair_{i}_boundary"] = {
            "type": "choice",
            "criteria": STAGE_BOUNDARY_CRITERIA,
            "instructions": (
                f"Compare state.pairs[{i}] (key={pairs[i]['key']!r}, value={pairs[i]['value']!r}) "
                f"to the pair right before it, state.pairs[{i - 1}] "
                f"(key={pairs[i - 1]['key']!r}, value={pairs[i - 1]['value']!r}). "
                "Does pair i begin a new stage of the task, or continue the same stage "
                "as pair i-1?"
            ),
        }
    return _call_with_retry(state, questions)


def pair_classification(decision: dict, index: int) -> str | None:
    """Reads back pair_<index>'s classification label. None if missing/malformed -
    callers should treat that the same as not_needed (skip it) rather than guess."""
    try:
        return decision["answers"][f"pair_{index}"]["choice"]
    except (KeyError, TypeError):
        return None


def pair_boundary(decision: dict, index: int) -> str | None:
    """Reads back whether pair_<index> starts a new stage. None if missing/malformed -
    callers should treat that as "same_stage" (merge rather than wrongly split)."""
    try:
        return decision["answers"][f"pair_{index}_boundary"]["choice"]
    except (KeyError, TypeError):
        return None
