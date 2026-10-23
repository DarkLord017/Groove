"""Persists field_pairs across sessions, once JEV has judged a run successful.

Keys are resolved against every fact already stored (not just the current session's)
via key_match.find_match, so the same fact typed under a different label still lands
on the same key instead of creating a duplicate.
"""
import os
import uuid

from qdrant_client import QdrantClient
from qdrant_client.http import models

from learn import key_match
from learn.errors import TransientFailure

QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
COLLECTION = "user_facts"

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


def save(key: str, value: str) -> str:
    """Resolves key against every stored fact, upserts {resolved_key: value}, and
    returns the resolved key (same as key if this is a genuinely new fact).
    """
    resolved_key = key_match.find_match(key, known_keys()) or key
    vector = key_match._bi_model.encode(resolved_key, normalize_embeddings=True).tolist()
    try:
        _client.upsert(
            collection_name=COLLECTION,
            points=[
                models.PointStruct(
                    id=_point_id(resolved_key),
                    vector=vector,
                    payload={"key": resolved_key, "value": value},
                )
            ],
        )
    except Exception as e:
        raise TransientFailure(f"facts save failed: {e}") from e
    return resolved_key
