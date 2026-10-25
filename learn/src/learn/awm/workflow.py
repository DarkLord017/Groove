import os
import time
import uuid

import httpx
import redis
from pydantic import BaseModel, Field, ValidationError
from qdrant_client import QdrantClient
from qdrant_client.http import models
from rapidfuzz import fuzz

from learn import key_match
from learn.errors import TransientFailure

OPENROUTER_BASE_URL = os.environ.get("OPENROUTER_BASE_URL", "https://openrouter.ai")
OPENROUTER_API_KEY = os.environ.get("OPENROUTER_API_KEY")
INDUCTION_MODEL = os.environ.get("AWM_INDUCTION_MODEL", "qwen/qwen-2.5-7b-instruct")
INDUCTION_TIMEOUT_SECONDS = 30.0
INDUCTION_MAX_ATTEMPTS = int(os.environ.get("AWM_INDUCTION_MAX_ATTEMPTS", "3"))
INDUCTION_BACKOFF_BASE_SECONDS = float(os.environ.get("AWM_INDUCTION_BACKOFF_BASE_SECONDS", "1"))
INDUCTION_MAX_TOKENS = 500  # without an explicit cap, some models default to requesting tens of
# thousands of tokens, which can trip this account's credit limit (hit live as a 402) for no reason -
# a workflow description + a handful of steps never needs anywhere near that.

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
COLLECTION = "site_workflows"
MAX_HISTORY = 4
SAME_WORKFLOW_THRESHOLD = key_match.RAPIDFUZZ_THRESHOLD

VISIT_WINDOW_SECONDS = 2 * 60 * 60  # 2 hours
VISIT_THRESHOLD = 3  # same number as tasks.py's PROMOTE_THRESHOLD, for consistency

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)
_qdrant = QdrantClient(url=QDRANT_URL)
if not _qdrant.collection_exists(COLLECTION):
    _qdrant.create_collection(
        collection_name=COLLECTION,
        vectors_config=models.VectorParams(
            size=key_match._bi_model.get_embedding_dimension(),
            distance=models.Distance.COSINE,
        ),
    )


class WorkflowStep(BaseModel):
    key: str
    value: str


class Workflow(BaseModel):
    description: str
    steps: list[WorkflowStep] = Field(min_length=2)  # the paper: "workflow must have at least two steps"


_WORKFLOW_SCHEMA = Workflow.model_json_schema()

# Adapted from the paper's own Appendix A.1 induction prompt - that version finds
# repeats ACROSS a list of tasks (the offline scenario); this is the online
# scenario's single-trajectory case (induce from THIS one successful run, nothing
# withheld waiting for a repeat - "If L_eval(e^t)=1: I(e^t) -> {w^t}").
SYSTEM_PROMPT = """Given a web navigation task's instruction and the actions taken to \
solve it, your task is to extract a reusable workflow.

Find the sub-routine of actions that accomplishes the task's goal, and represent it \
as a workflow with a short natural-language description and an ordered list of \
steps. Represent the non-fixed elements (the actual values typed or selected, e.g. a \
city name, a date, a price) with descriptive variable names in curly braces, e.g. \
"{origin-city}" instead of "mumbai" - the workflow should generalize to the same \
kind of task with different values, not just this one run. Leave fixed, structural \
values (like "yes" for a checkbox) as-is.

Do not invent steps that weren't in the given actions.
"""


def _post_once(objective: str, pairs: list[dict]) -> dict:
    numbered = "\n".join(f"{i}: key={p['key']!r}, value={p['value']!r}" for i, p in enumerate(pairs))
    user_content = f'Task instruction: "{objective}"\n\nActions taken, in order:\n{numbered}'

    try:
        response = httpx.post(
            f"{OPENROUTER_BASE_URL}/api/v1/chat/completions",
            headers={"Authorization": f"Bearer {OPENROUTER_API_KEY}"},
            json={
                "model": INDUCTION_MODEL,
                "max_tokens": INDUCTION_MAX_TOKENS,
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": user_content},
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {"name": "workflow", "strict": True, "schema": _WORKFLOW_SCHEMA},
                },
            },
            timeout=INDUCTION_TIMEOUT_SECONDS,
        )
    except httpx.TransportError as e:
        raise TransientFailure(f"awm workflow induction request failed: {e}") from e

    if response.status_code >= 500:
        raise TransientFailure(f"awm workflow induction returned {response.status_code}: {response.text}")
    response.raise_for_status()  # a 4xx won't be fixed by retrying (we hit this live - a credit-limited 402)
    return response.json()


def _call_with_retry(objective: str, pairs: list[dict]) -> dict:
    last_error: TransientFailure | None = None
    for attempt in range(1, INDUCTION_MAX_ATTEMPTS + 1):
        try:
            return _post_once(objective, pairs)
        except TransientFailure as e:
            last_error = e
            if attempt < INDUCTION_MAX_ATTEMPTS:
                time.sleep(INDUCTION_BACKOFF_BASE_SECONDS * (2 ** (attempt - 1)))
    raise TransientFailure(f"awm workflow induction failed after {INDUCTION_MAX_ATTEMPTS} attempts: {last_error}") from last_error


def _visits_key(domain: str) -> str:
    return f"awm:visits:{domain}"


def should_induce(domain: str) -> bool:
    """Records this successful run as a visit to domain (a Redis sorted set keyed by
    timestamp, old entries trimmed out), and returns whether the domain has been
    visited at least VISIT_THRESHOLD times within the last VISIT_WINDOW_SECONDS. A
    one-off success on a site that's likely never visited again shouldn't cost an
    LLM call or accumulate a workflow entry that will probably never be reused -
    only sites the user is actually frequenting get induced at all.
    """
    domain = domain.lower()
    key = _visits_key(domain)
    now = time.time()
    try:
        _client.zadd(key, {str(uuid.uuid4()): now})
        _client.zremrangebyscore(key, 0, now - VISIT_WINDOW_SECONDS)
        _client.expire(key, VISIT_WINDOW_SECONDS)
        count = _client.zcard(key)
    except redis.RedisError as e:
        raise TransientFailure(f"awm should_induce failed: {e}") from e
    return count >= VISIT_THRESHOLD


def induce_workflow(objective: str, pairs: list[dict]) -> Workflow | None:
    if len(pairs) < 2:
        return None
    if not OPENROUTER_API_KEY:
        raise RuntimeError("OPENROUTER_API_KEY is not set - cannot call the induction model")

    data = _call_with_retry(objective, pairs)
    try:
        content = data["choices"][0]["message"]["content"]
        return Workflow.model_validate_json(content)
    except (KeyError, IndexError, TypeError, ValidationError):
        return None


def _point_id(key: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, key))


def known_workflows(domain: str) -> list[dict]:
    """Every distinct workflow known for this domain (post-dedup - see
    record_workflow). The paper injects ALL of a site's workflows into context (no
    retrieval/selection step is described), so this returns the whole set, not a
    top-k search - but "the whole set" now means distinct workflows, not one entry
    per successful run.
    """
    domain = domain.lower()
    points, _ = _qdrant.scroll(
        collection_name=COLLECTION,
        scroll_filter=models.Filter(must=[models.FieldCondition(key="domain", match=models.MatchValue(value=domain))]),
        limit=10_000,
        with_payload=True,
        with_vectors=False,
    )
    return [p.payload for p in points]


def _occurrence_values(steps: list[dict]) -> str:
    return "; ".join(s["value"] for s in steps)


def record_workflow(domain: str, workflow: Workflow) -> None:
    """Saves an induced workflow into this domain's workflow memory. The paper's own
    online rule is pure append ("M^t + {w^t} -> M^(t+1)", no merge/dedup at all),
    but that's a real gap in the paper's own design for a system that runs
    indefinitely rather than over a bounded benchmark - so this deliberately departs
    from it: a new workflow's description is resolved against existing ones for this
    domain via key_match (same pattern as hmt/site_tree.py's _resolve_stage_label),
    and a close-enough match updates that entry (bumping its count, rolling a bounded
    history of distinct step-sequences) instead of creating a near-duplicate point.
    """
    domain = domain.lower()
    existing_descriptions = [w["description"] for w in known_workflows(domain)]
    resolved_description = key_match.find_match(workflow.description, existing_descriptions) or workflow.description

    point_id = _point_id(f"{domain}|{resolved_description}")
    existing = _qdrant.retrieve(collection_name=COLLECTION, ids=[point_id], with_payload=True)
    history = existing[0].payload.get("steps_history", []) if existing else []
    count = (existing[0].payload.get("count", 0) if existing else 0) + 1

    steps = [s.model_dump() for s in workflow.steps]
    values = _occurrence_values(steps)
    if not history or fuzz.ratio(values, _occurrence_values(history[-1]["steps"])) < SAME_WORKFLOW_THRESHOLD:
        history.append({"steps": steps, "timestamp": time.time()})
        history = history[-MAX_HISTORY:]

    vector = key_match._bi_model.encode(f"{domain}: {resolved_description}", normalize_embeddings=True).tolist()
    _qdrant.upsert(collection_name=COLLECTION, points=[models.PointStruct(
        id=point_id,
        vector=vector,
        payload={
            "domain": domain,
            "description": resolved_description,
            "steps_history": history,
            "count": count,
            "timestamp": time.time(),
        },
    )])
