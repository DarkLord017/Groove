"""Client for Jev via OpenRouter's decisions endpoint (not jevai.org directly - OpenRouter
hosts the real typed-decisions model at /api/alpha/decisions, confirmed working with a
real OPENROUTER_API_KEY; the model only works there, not on /chat/completions).
Called once per run, from TurnCompleteHandler, with the objective/steps built up over
the session by jev_session plus the run's own final message.
"""

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


def review_completion(objective: str, steps: list[str], agent_message: str) -> dict:
    """Asks Jev for a single "success" probability (0-1) given the objective, the
    actual recorded steps, and the agent's own final message. Steps are weighted over
    the agent's self-report in the instructions, since an agent's last message can
    overclaim relative to what it actually did.

    Retries with exponential backoff on transient failures (network errors, 5xx). A 4xx
    (malformed request) is raised immediately - retrying it won't help.
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
                "if they disagree."
            ),
        }
    }

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
