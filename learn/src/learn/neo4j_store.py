import json
import os

from neo4j import GraphDatabase
from neo4j.exceptions import ServiceUnavailable

from learn.errors import TransientFailure

NEO4J_URI = os.environ.get("NEO4J_URI", "bolt://127.0.0.1:7687")
NEO4J_USER = os.environ.get("NEO4J_USER", "neo4j")
NEO4J_PASSWORD = os.environ.get("NEO4J_PASSWORD", "password")

_driver = GraphDatabase.driver(NEO4J_URI, auth=(NEO4J_USER, NEO4J_PASSWORD))

# Simplest possible schema to start: every message becomes an Event node linked to its
# Session node. This is a placeholder for a real graph schema per kind - the point right
# now is proving the pipeline (RabbitMQ -> learn -> Neo4j) end to end, not the schema.
_STORE_EVENT_QUERY = """
MERGE (s:Session {id: $session_id})
CREATE (e:Event {kind: $kind, label: $label, payload: $payload, recorded_at: datetime()})
CREATE (e)-[:BELONGS_TO]->(s)
"""


def store_event(session_id: str, kind: str, label: str, payload: dict) -> None:
    try:
        with _driver.session() as session:
            session.run(
                _STORE_EVENT_QUERY,
                session_id=session_id,
                kind=kind,
                label=label,
                payload=json.dumps(payload),
            )
    except ServiceUnavailable as e:
        raise TransientFailure(str(e)) from e
