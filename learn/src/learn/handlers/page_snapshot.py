from learn.handlers.base import EventHandler
from learn.neo4j_store import store_event


class PageSnapshotHandler(EventHandler):
    KIND = "page.snapshot"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        store_event(session_id, self.KIND, label, payload)
