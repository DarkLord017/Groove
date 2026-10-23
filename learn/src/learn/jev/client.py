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


def classify_pairs(pairs: list[dict]) -> dict:
    """Asks Jev to classify each {key, value} pair as personal / task / not_needed.
    One call covers every pair (questions pair_0, pair_1, ...) instead of one call
    per pair. Returns the raw decision dict - read it back with pair_classification.
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
    return _call_with_retry(state, questions)


def pair_classification(decision: dict, index: int) -> str | None:
    """Reads back pair_<index>'s classification label. None if missing/malformed -
    callers should treat that the same as not_needed (skip it) rather than guess."""
    try:
        return decision["answers"][f"pair_{index}"]["choice"]
    except (KeyError, TypeError):
        return None
