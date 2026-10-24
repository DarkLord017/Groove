"""Per-domain Intent -> Stage memory tree, built from successful runs' field_pairs.

Inspired by Agent Workflow Memory (induce reusable routines from successful
trajectories) and the Hierarchical Memory Tree (decouple the logical plan - what
subgoals a task breaks into - from site-specific execution). Here:

- Intent: the task's goal on this domain (e.g. "search flight mumbai to delhi"),
  resolved via key_match the same way facts/tasks resolve keys - so differently
  phrased objectives for the same underlying task collapse onto one node.
- Stage: one of jev.STAGE_CRITERIA's fixed, reusable subgoal categories (not free
  text) - a run's field_pairs are classified into stages and grouped into
  contiguous occurrences, each occurrence storing the ordered {key, value} pairs
  that happened during it (already semantic descriptions, not raw refs - see
  capture/elements.rs's describe()).

Action-level detail lives inline on each Stage occurrence rather than in its own
collection, since nothing retrieves an action independently of its stage.
"""
import os
import time
import uuid
from urllib.parse import urlparse

import redis
from qdrant_client import QdrantClient
from qdrant_client.http import models
from rapidfuzz import fuzz

from learn import key_match
from learn.errors import TransientFailure
from learn.jev import client as jev

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
TTL_SECONDS = 60 * 60

INTENTS_COLLECTION = "site_intents"
STAGES_COLLECTION = "site_stages"
MAX_HISTORY = 4
SAME_OCCURRENCE_THRESHOLD = key_match.RAPIDFUZZ_THRESHOLD

_client = redis.Redis.from_url(REDIS_URL, decode_responses=True)
_qdrant = QdrantClient(url=QDRANT_URL)

for _collection in (INTENTS_COLLECTION, STAGES_COLLECTION):
    if not _qdrant.collection_exists(_collection):
        _qdrant.create_collection(
            collection_name=_collection,
            vectors_config=models.VectorParams(
                size=key_match._bi_model.get_embedding_dimension(),
                distance=models.Distance.COSINE,
            ),
        )


def _point_id(key: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, key))


def _domain_key(session_id: str) -> str:
    return f"site:domain:{session_id}"


def set_domain(session_id: str, url: str) -> None:
    host = (urlparse(url).hostname or "").removeprefix("www.").lower()
    if not host:
        return
    try:
        _client.set(_domain_key(session_id), host, ex=TTL_SECONDS)
    except redis.RedisError as e:
        raise TransientFailure(f"site_tree set_domain failed: {e}") from e


def get_domain(session_id: str) -> str:
    try:
        return _client.get(_domain_key(session_id)) or "unknown"
    except redis.RedisError as e:
        raise TransientFailure(f"site_tree get_domain failed: {e}") from e


def clear_domain(session_id: str) -> None:
    try:
        _client.delete(_domain_key(session_id))
    except redis.RedisError as e:
        raise TransientFailure(f"site_tree clear_domain failed: {e}") from e


def _known_intents_key(domain: str) -> str:
    return f"site:intents:{domain}"


def _known_intents(domain: str) -> list[str]:
    try:
        return list(_client.smembers(_known_intents_key(domain)))
    except redis.RedisError as e:
        raise TransientFailure(f"site_tree _known_intents failed: {e}") from e


def _resolve_intent(domain: str, objective: str) -> str:
    objective = objective.lower().strip()
    if not objective:
        return "unspecified"

    resolved = key_match.find_match(objective, _known_intents(domain)) or objective
    try:
        _client.sadd(_known_intents_key(domain), resolved)
    except redis.RedisError as e:
        raise TransientFailure(f"site_tree _resolve_intent failed: {e}") from e

    vector = key_match._bi_model.encode(f"{domain}: {resolved}", normalize_embeddings=True).tolist()
    _qdrant.upsert(collection_name=INTENTS_COLLECTION, points=[models.PointStruct(
        id=_point_id(f"{domain}|{resolved}"),
        vector=vector,
        payload={"domain": domain, "intent": resolved},
    )])
    return resolved


def _segment_stages(pairs: list[dict]) -> list[tuple[str, list[dict]]]:
    """Classifies each field_pairs entry into a fixed stage label, then groups
    consecutive same-label pairs into one occurrence of that stage, in order.
    """
    if not pairs:
        return []
    decision = jev.classify_stages(pairs)
    segments: list[tuple[str, list[dict]]] = []
    for i, pair in enumerate(pairs):
        label = jev.stage_classification(decision, i) or "other"
        if segments and segments[-1][0] == label:
            segments[-1][1].append(pair)
        else:
            segments.append((label, [pair]))
    return segments


def _occurrence_values(actions: list[dict]) -> str:
    # Compare only the values, not the key labels - the labels are shared/long across
    # occurrences of the same stage (e.g. "textbox \"origin...\"") and would dominate
    # the similarity ratio, masking a genuinely different occurrence (different
    # values) as a duplicate of the last one.
    return "; ".join(a["value"] for a in actions)


def _save_stage(domain: str, intent: str, stage: str, pre: str, post: str, actions: list[dict]) -> None:
    point_id = _point_id(f"{domain}|{intent}|{stage}")
    existing = _qdrant.retrieve(collection_name=STAGES_COLLECTION, ids=[point_id], with_payload=True)
    occurrences = existing[0].payload.get("occurrences", []) if existing else []
    count = (existing[0].payload.get("count", 0) if existing else 0) + 1

    values = _occurrence_values(actions)
    if not occurrences or fuzz.ratio(values, _occurrence_values(occurrences[-1]["actions"])) < SAME_OCCURRENCE_THRESHOLD:
        occurrences.append({"actions": actions, "timestamp": time.time()})
        occurrences = occurrences[-MAX_HISTORY:]

    vector = key_match._bi_model.encode(f"{domain} | {intent} | {stage}", normalize_embeddings=True).tolist()
    _qdrant.upsert(collection_name=STAGES_COLLECTION, points=[models.PointStruct(
        id=point_id,
        vector=vector,
        payload={
            "domain": domain,
            "intent": intent,
            "stage": stage,
            "pre_condition": pre,
            "post_condition": post,
            "occurrences": occurrences,
            "count": count,
        },
    )])


def record_run(domain: str, objective: str, pairs: list[dict]) -> None:
    """Called once per successful run (from JudgeResultHandler), on the same
    field_pairs a run's personal/task classification already looked at. Resolves the
    run's Intent node, segments its pairs into stage occurrences, and upserts each
    Stage - a new stage name creates a node, a repeat increments its count and rolls
    a bounded history of example occurrences.
    """
    domain = domain.lower()
    intent = _resolve_intent(domain, objective)
    segments = _segment_stages(pairs)
    for idx, (stage, actions) in enumerate(segments):
        pre = segments[idx - 1][0] if idx > 0 else "start"
        post = segments[idx + 1][0] if idx + 1 < len(segments) else "end"
        _save_stage(domain, intent, stage, pre, post, actions)
