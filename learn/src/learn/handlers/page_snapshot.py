from learn import jev_session
from learn.handlers.base import EventHandler
from learn.neo4j_store import store_event


class PageSnapshotHandler(EventHandler):
    KIND = "page.snapshot"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        jev_session.append_work(session_id, jev_session.summarize_step(label, payload))
        store_event(session_id, self.KIND, label, payload)
