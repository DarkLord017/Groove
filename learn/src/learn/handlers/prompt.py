from learn.handlers.base import EventHandler
from learn.jev import session as jev_session


class PromptHandler(EventHandler):
    KIND = "prompt"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        jev_session.set_objective(session_id, payload.get("prompt", ""))
