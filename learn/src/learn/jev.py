import os
import time

import httpx

from learn.errors import TransientFailure

JEV_BASE_URL = os.environ.get("JEV_BASE_URL", "https://www.jevai.org")
JEV_API_KEY = os.environ.get("JEV_API_KEY")
JEV_TIMEOUT_SECONDS = 10.0
JEV_MAX_ATTEMPTS = int(os.environ.get("JEV_MAX_ATTEMPTS", "4"))
JEV_BACKOFF_BASE_SECONDS = float(os.environ.get("JEV_BACKOFF_BASE_SECONDS", "1"))


def _post_once(objective: str, completed_work: str) -> dict:
    try:
        response = httpx.post(
            f"{JEV_BASE_URL}/api/v1/decisions/completion",
            headers={"Authorization": f"Bearer {JEV_API_KEY}"},
            json={"objective": objective, "completed_work": completed_work},
            timeout=JEV_TIMEOUT_SECONDS,
        )
    except httpx.TransportError as e:
        raise TransientFailure(f"JEV request failed: {e}") from e

    if response.status_code >= 500:
        raise TransientFailure(f"JEV returned {response.status_code}: {response.text}")
    response.raise_for_status()  #
    return response.json()


def review_completion(objective: str, completed_work: str) -> dict:
    """Calls POST /api/v1/decisions/completion with exponential backoff on transient
    failures (network errors, 5xx). Returns the parsed response, e.g.
    {"decision": "complete" | "verify_more" | "incomplete", ...}.
    """
    if not JEV_API_KEY:
        raise RuntimeError("JEV_API_KEY is not set - cannot call JEV")

    last_error: TransientFailure | None = None
    for attempt in range(1, JEV_MAX_ATTEMPTS + 1):
        try:
            return _post_once(objective, completed_work)
        except TransientFailure as e:
            last_error = e
            if attempt < JEV_MAX_ATTEMPTS:
                backoff = JEV_BACKOFF_BASE_SECONDS * (2 ** (attempt - 1))
                print(f"learn: jev call failed (attempt {attempt}/{JEV_MAX_ATTEMPTS}): {e} - retrying in {backoff}s")
                time.sleep(backoff)

    raise TransientFailure(f"JEV call failed after {JEV_MAX_ATTEMPTS} attempts: {last_error}") from last_error
