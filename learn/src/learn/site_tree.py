import os
import time
import uuid
from urllib.parse import urlparse

import redis
import spacy
from qdrant_client import QdrantClient
from qdrant_client.http import models
from rapidfuzz import fuzz

from learn import key_match
from learn.errors import TransientFailure
from learn.jev import client as jev

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
TTL_SECONDS = 60 * 60

_nlp = spacy.load("en_core_web_sm")

# Navigating to the site itself isn't part of the task - domain is already tracked
# separately (set_domain/get_domain) - so a verb whose only job is "go to the site"
# shouldn't contribute anything to the intent text.
_NAV_VERBS = {"go", "navigate", "open", "visit"}

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


def _known_intents(domain: str) -> list[str]:
    """Every intent already resolved for this domain. Every resolved intent is
    upserted into Qdrant unconditionally (no promotion threshold like tasks.py's
    frequency counter), so Qdrant alone is the full source of truth here - same as
    facts.py's known_keys(), no separate Redis set needed.
    """
    points, _ = _qdrant.scroll(
        collection_name=INTENTS_COLLECTION,
        scroll_filter=models.Filter(must=[models.FieldCondition(key="domain", match=models.MatchValue(value=domain))]),
        limit=10_000,
        with_payload=True,
        with_vectors=False,
    )
    return [p.payload["intent"] for p in points]


def _intent_object(verb) -> tuple:
    """Finds what the verb acts on, as either a direct object (dobj) or, for a
    verb+preposition pairing like "search for"/"ask about", the object of that
    preposition - but only the preposition directly on the verb, never one further
    down modifying the object itself (that's where the variable city/date/etc. live,
    e.g. "flights" is search's pobj via "for", while "from"/"to" are flights' own
    prep children, not search's - verified against real spaCy dependency parses).
    Returns (object_token, preposition_word_or_None), or (None, None).
    """
    dobj = next((c for c in verb.children if c.dep_ == "dobj"), None)
    if dobj is not None:
        return dobj, None
    prep = next((c for c in verb.children if c.dep_ == "prep"), None)
    if prep is not None:
        pobj = next((c for c in prep.children if c.dep_ == "pobj"), None)
        if pobj is not None:
            return pobj, prep.text
    return None, None


def _extract_intent_core(objective: str) -> str | None:
    """Reduces a raw prompt to its grammatical core (verb + object, e.g. "look up
    flights" or "search for cheap flights") so that two prompts differing only in
    which city/date/site fills the variable slots resolve to the same intent text.
    Verified against real eval prompts - NER-based stripping (spaCy's small model
    misses plenty of city names) isn't reliable enough for this, but the dependency
    structure of "look up flights [on <date> <city> to <city>]" is consistent even
    when the model has no idea what the trailing proper nouns are.
    """
    phrases = []
    for tok in _nlp(objective):
        if tok.pos_ != "VERB" or tok.lemma_ in _NAV_VERBS:
            continue
        obj, prep_word = _intent_object(tok)
        if obj is None:
            continue
        prt = next((c for c in tok.children if c.dep_ == "prt"), None)
        mods = [c.text for c in obj.children if c.dep_ in ("amod", "compound", "nummod")]
        parts = [tok.lemma_, *([prt.text] if prt else []), *([prep_word] if prep_word else []), *mods, obj.text]
        phrases.append(" ".join(parts))
    return " / ".join(phrases) or None


def _resolve_intent(domain: str, objective: str) -> str:
    objective = objective.lower().strip()
    if not objective:
        return "unspecified"
    objective = _extract_intent_core(objective) or objective

    resolved = key_match.find_match(objective, _known_intents(domain)) or objective
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


# HMT's Planner/Actor constants, used as-is from the paper.
STAGE_CANDIDATES_K = 8  # K_G
COND_MATCH_WEIGHT = 0.3  # lambda
MIN_MARGIN = 0.1  # delta: top1 - top2 must clear this
MIN_CONFIDENCE = 0.15  # tau: top1 itself must clear this


def _cond_match(pre_condition: str, completed_stages: list[str]) -> float:
    """HMT's CondMatch is a Jaccard overlap between free-text conditions. Our Stage
    labels are categorical, not free text, so the faithful version of "does this
    candidate's precondition hold" is exact set membership rather than fuzzy token
    overlap: 1.0 if its pre_condition already happened (or is "start" and nothing has
    happened yet), 0.0 otherwise.
    """
    if pre_condition == "start":
        return 1.0 if not completed_stages else 0.0
    return 1.0 if pre_condition in completed_stages else 0.0


def select_stage(domain: str, intent: str, completed_stages: list[str]) -> tuple[dict | None, float]:
    """The Planner: given what's already been done this run (completed_stages, in
    order), picks which Stage node is most likely next. Returns (None, confidence) if
    the top candidate isn't clearly ahead of the runner-up or isn't confident enough
    on its own - callers should fall back to no-hint rather than guess. Not wired to
    any hook yet - this is the selection logic on its own, for recall to call later.
    """
    domain = domain.lower()
    query_vector = key_match._bi_model.encode(f"{domain} | {intent}", normalize_embeddings=True).tolist()
    hits = _qdrant.query_points(
        collection_name=STAGES_COLLECTION,
        query=query_vector,
        query_filter=models.Filter(must=[
            models.FieldCondition(key="domain", match=models.MatchValue(value=domain)),
            models.FieldCondition(key="intent", match=models.MatchValue(value=intent)),
        ]),
        limit=STAGE_CANDIDATES_K,
    ).points

    # A stage already done this run isn't a candidate for "what's next" - without this,
    # an already-finished stage can still score cond_match=1.0 (its OWN precondition
    # is satisfied) and tie with the real next stage.
    candidates = [hit for hit in hits if hit.payload["stage"] not in completed_stages]
    if not candidates:
        return None, 0.0

    scored = sorted(
        (
            ((1 - COND_MATCH_WEIGHT) * hit.score + COND_MATCH_WEIGHT * _cond_match(hit.payload["pre_condition"], completed_stages), hit.payload)
            for hit in candidates
        ),
        key=lambda s: s[0],
        reverse=True,
    )

    top_score, top_payload = scored[0]
    runner_up_score = scored[1][0] if len(scored) > 1 else top_score
    margin = top_score - runner_up_score

    if top_score < MIN_CONFIDENCE or (len(scored) > 1 and margin < MIN_MARGIN):
        return None, top_score

    return top_payload, top_score


def ground_action(stored_label: str, live_labels: list[str]) -> str | None:
    """The Actor's grounding step: a Stage's stored action used some semantic
    descriptor (e.g. textbox "Origin City or Airport") on a PAST page load - refs
    themselves are dead on a new page load, but the descriptor isn't, so find_match
    re-resolves it against the CURRENT page's live labels. None if nothing on the
    current page matches closely enough.
    """
    return key_match.find_match(stored_label, live_labels)


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
