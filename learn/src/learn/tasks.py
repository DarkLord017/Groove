import os
import uuid

import redis
from qdrant_client import QdrantClient
from qdrant_client.http import models

from learn import key_match
from learn.errors import TransientFailure

REDIS_URL = os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/")
TASKS_KEY = "tasks:counts"

QDRANT_URL = os.environ.get("QDRANT_URL", "http://127.0.0.1:6333")
COLLECTION = "user_routines"
PROMOTE_THRESHOLD = int(os.environ.get("TASK_PROMOTE_THRESHOLD", "3"))

_redis = redis.Redis.from_url(REDIS_URL, decode_responses=True)
_qdrant = QdrantClient(url=QDRANT_URL)

if not _qdrant.collection_exists(COLLECTION):
    _qdrant.create_collection(
        collection_name=COLLECTION,
        vectors_config=models.VectorParams(
            size=key_match._bi_model.get_embedding_dimension(),
            distance=models.Distance.COSINE,
        ),
    )


def _point_id(task: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, task))


def known_tasks() -> list[str]:
    try:
        return list(_redis.hkeys(TASKS_KEY))
    except redis.RedisError as e:
        raise TransientFailure(f"tasks known_tasks failed: {e}") from e


def observe(objective: str) -> None:
    """Resolves objective against every task seen before, bumps its count, and
    promotes it to Qdrant once it's recurred often enough. No-op for an empty
    objective.
    """
    if not objective:
        return

    objective = objective.lower()
    resolved = key_match.find_match(objective, known_tasks()) or objective
    try:
        count = _redis.hincrby(TASKS_KEY, resolved, 1)
    except redis.RedisError as e:
        raise TransientFailure(f"tasks observe failed: {e}") from e

    if count < PROMOTE_THRESHOLD:
        return

    vector = key_match._bi_model.encode(resolved, normalize_embeddings=True).tolist()
    try:
        _qdrant.upsert(
            collection_name=COLLECTION,
            points=[
                models.PointStruct(
                    id=_point_id(resolved),
                    vector=vector,
                    payload={"task": resolved, "count": count},
                )
            ],
        )
    except Exception as e:
        raise TransientFailure(f"tasks promote failed: {e}") from e
