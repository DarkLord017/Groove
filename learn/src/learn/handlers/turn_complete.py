from learn import jev, jev_session
from learn.errors import TransientFailure
from learn.handlers.base import EventHandler
from learn.neo4j_store import store_event


class TurnCompleteHandler(EventHandler):
    KIND = "turn.complete"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        last_assistant_message = payload.get("last_assistant_message", "")
        jev_session.append_work(session_id, f"{label}: {last_assistant_message}")

        objective, completed_work = jev_session.peek(session_id)
        try:
            decision = jev.review_completion(objective, "\n".join(completed_work))
        except TransientFailure:
            # Leave the accumulated state in place - a message requeue (consumer.py's
            # own backoff) will retry with full context instead of an empty one.
            raise
        except Exception:
            jev_session.clear(session_id)
            raise

        jev_session.clear(session_id)
        print(f"learn: jev decision for session={session_id}: {decision}")

        store_event(session_id, self.KIND, label, payload)
        store_event(session_id, "judge.result", "jev_review_completion", decision)
