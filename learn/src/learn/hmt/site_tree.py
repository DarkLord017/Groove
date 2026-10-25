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
from learn.hmt import stage_llm

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


KEY_OVERLAP_THRESHOLD = 0.5  # verified on real data: benchmarks/stage_key_overlap_benchmark.py, 7/7


def _jaccard_keys(a: set, b: set) -> float:
    if not a or not b:
        return 0.0
    return len(a & b) / len(a | b)


def _known_stage_keysets(domain: str, intent: str) -> list[tuple[str, set]]:
    """Every existing stage's (point_id, key-set) for this domain+intent - the
    key-set is the fields touched by its most recent occurrence, used as the
    stage's identity signature for matching (see _resolve_stage_id). Keyed by the
    point's own id, NOT its display name - two different stages can coincidentally
    get the same AI-written name (verified live), and using the name as identity
    would wrongly collide them in storage.
    """
    points, _ = _qdrant.scroll(
        collection_name=STAGES_COLLECTION,
        scroll_filter=models.Filter(must=[
            models.FieldCondition(key="domain", match=models.MatchValue(value=domain)),
            models.FieldCondition(key="intent", match=models.MatchValue(value=intent)),
        ]),
        limit=10_000,
        with_payload=True,
        with_vectors=False,
    )
    result = []
    for p in points:
        occurrences = p.payload.get("occurrences", [])
        if occurrences:
            result.append((p.id, {a["key"] for a in occurrences[-1]["actions"]}))
    return result


def _resolve_stage_id(domain: str, intent: str, actions: list[dict]) -> str:
    """A stage's identity (its Qdrant point id) is decided by which FIELDS it
    actually touches, not by comparing stage_llm's free-text name. Verified on real
    data (benchmarks/stage_key_overlap_benchmark.py, 7/7, including the case
    name-matching would get wrong: two genuinely different stages whose AI-written
    name happened to be identical text) that key_match.find_match - reliable for
    field labels and intents - is NOT reliable for this short, freely-reworded text
    (benchmarks/key_match_sentence_benchmark.py, 4/9). Mints a fresh id when no
    existing stage's fields overlap closely enough - i.e. this really is new.
    """
    new_keys = {a["key"] for a in actions}
    best_id, best_score = None, 0.0
    for point_id, keyset in _known_stage_keysets(domain, intent):
        score = _jaccard_keys(new_keys, keyset)
        if score > best_score:
            best_id, best_score = point_id, score
    if best_score >= KEY_OVERLAP_THRESHOLD:
        return best_id
    return str(uuid.uuid4())


def _occurrence_values(actions: list[dict]) -> str:
    # Compare only the values, not the key labels - the labels are shared/long across
    # occurrences of the same stage (e.g. "textbox \"origin...\"") and would dominate
    # the similarity ratio, masking a genuinely different occurrence (different
    # values) as a duplicate of the last one.
    return "; ".join(a["value"] for a in actions)


def _save_stage(point_id: str, domain: str, intent: str, stage: str, pre: str, post: str, actions: list[dict]) -> None:
    existing = _qdrant.retrieve(collection_name=STAGES_COLLECTION, ids=[point_id], with_payload=True)
    occurrences = existing[0].payload.get("occurrences", []) if existing else []
    count = (existing[0].payload.get("count", 0) if existing else 0) + 1

    values = _occurrence_values(actions)
    if not occurrences or fuzz.ratio(values, _occurrence_values(occurrences[-1]["actions"])) < SAME_OCCURRENCE_THRESHOLD:
        occurrences.append({"actions": actions, "timestamp": time.time()})
        occurrences = occurrences[-MAX_HISTORY:]

    # Deliberately no domain in the embedded text - select_stage searches without a
    # domain filter (see its docstring), so a stage's vector should represent what
    # it IS (intent + stage), not where it was learned - domain still lives in the
    # payload below for storage/merge identity, just not folded into the meaning
    # this vector is meant to capture.
    vector = key_match._bi_model.encode(f"{intent} | {stage}", normalize_embeddings=True).tolist()
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


def _jaccard(a: str, b: str) -> float:
    tokens_a, tokens_b = set(a.lower().split()), set(b.lower().split())
    if not tokens_a or not tokens_b:
        return 0.0
    return len(tokens_a & tokens_b) / len(tokens_a | tokens_b)


def _cond_match(pre_condition: str, completed_post_conditions: list[str]) -> float:
    """HMT's actual CondMatch: Jaccard word-overlap between a candidate's
    pre_condition sentence and the post_condition sentences of stages already
    completed this run - real text comparison, now that stage_llm gives us real
    condition sentences instead of a neighbor-stage name. Empty (nothing completed
    yet, e.g. the first stage of a run) scores 0 - nothing to match against, so
    selection falls back to pure semantic similarity for that case.
    """
    if not completed_post_conditions:
        return 0.0
    return max(_jaccard(pre_condition, cond) for cond in completed_post_conditions)


def select_stage(intent: str, completed: list[dict]) -> tuple[dict | None, float]:
    completed_names = [c["stage"] for c in completed]
    completed_posts = [c["post_condition"] for c in completed]

    query_vector = key_match._bi_model.encode(intent, normalize_embeddings=True).tolist()
    hits = _qdrant.query_points(
        collection_name=STAGES_COLLECTION,
        query=query_vector,
        query_filter=models.Filter(must=[
            models.FieldCondition(key="intent", match=models.MatchValue(value=intent)),
        ]),
        limit=STAGE_CANDIDATES_K,
    ).points

    # A stage already done this run isn't a candidate for "what's next" - without this,
    # an already-finished stage can still score cond_match=1.0 (its OWN precondition
    # is satisfied) and tie with the real next stage.
    candidates = [hit for hit in hits if hit.payload["stage"] not in completed_names]
    if not candidates:
        return None, 0.0

    scored = sorted(
        (
            ((1 - COND_MATCH_WEIGHT) * hit.score + COND_MATCH_WEIGHT * _cond_match(hit.payload["pre_condition"], completed_posts), hit.payload)
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
    """Called once per successful run (from JudgeResultHandler) on the run's
    field_pairs. Resolves the run's Intent node, then asks stage_llm - a real LLM
    call over OpenRouter, not Jev's bounded decisions API - to segment the pairs
    into stages with real semantic names and pre/post-condition sentences (HMT's
    actual Subgoal Segmentation step). Each stage's proposed name is resolved
    against previously stored stages for this domain+intent before saving, so
    repeats collapse onto the same node instead of fragmenting by wording.
    Skips recording entirely if the segmenter call fails or returns something
    malformed - better to record nothing than record a broken structure.
    """
    domain = domain.lower()
    intent = _resolve_intent(domain, objective)

    segments = stage_llm.segment_pairs(intent, pairs)
    if not segments:
        return

    for segment in segments:
        actions = [pairs[i] for i in segment.pair_indices]
        point_id = _resolve_stage_id(domain, intent, actions)
        _save_stage(point_id, domain, intent, segment.name, segment.pre_condition, segment.post_condition, actions)
