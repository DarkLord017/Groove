import os
import time
import uuid

from qdrant_client import QdrantClient
from qdrant_client.http import models
from rapidfuzz import fuzz

from learn import key_match
from learn.errors import TransientFailure

QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
COLLECTION = "user_facts"
MAX_HISTORY = 4
SAME_VALUE_THRESHOLD = key_match.RAPIDFUZZ_THRESHOLD  # reuse the same "basically identical" bar

_client = QdrantClient(url=QDRANT_URL)

if not _client.collection_exists(COLLECTION):
    _client.create_collection(
        collection_name=COLLECTION,
        vectors_config=models.VectorParams(
            size=key_match._bi_model.get_embedding_dimension(),
            distance=models.Distance.COSINE,
        ),
    )


def _point_id(key: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, key))


def known_keys() -> list[str]:
    try:
        points, _ = _client.scroll(collection_name=COLLECTION, limit=10_000, with_payload=True, with_vectors=False)
    except Exception as e:
        raise TransientFailure(f"facts known_keys failed: {e}") from e
    return [p.payload["key"] for p in points]


def _existing_values(resolved_key: str) -> list[dict]:
    try:
        points = _client.retrieve(collection_name=COLLECTION, ids=[_point_id(resolved_key)], with_payload=True)
    except Exception as e:
        raise TransientFailure(f"facts _existing_values failed: {e}") from e
    if not points:
        return []
    return points[0].payload.get("values", [])


def save(key: str, value: str) -> str:
    """Resolves key against every stored fact, appends {value, timestamp} to its
    history, and returns the resolved key (same as key if this is a genuinely new
    fact). No-ops the append if value is basically the same string as the most recent
    entry (formatting difference only, not a real change).
    """
    value = value.lower()
    resolved_key = key_match.find_match(key, known_keys()) or key
    history = _existing_values(resolved_key)

    if history and fuzz.ratio(value, history[-1]["value"]) >= SAME_VALUE_THRESHOLD:
        return resolved_key

    history.append({"value": value, "timestamp": time.time()})
    history = history[-MAX_HISTORY:]

    vector = key_match._bi_model.encode(resolved_key, normalize_embeddings=True).tolist()
    try:
        _client.upsert(
            collection_name=COLLECTION,
            points=[
                models.PointStruct(
                    id=_point_id(resolved_key),
                    vector=vector,
                    payload={"key": resolved_key, "values": history},
                )
            ],
        )
    except Exception as e:
        raise TransientFailure(f"facts save failed: {e}") from e
    return resolved_key
