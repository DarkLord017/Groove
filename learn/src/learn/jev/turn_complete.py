from learn import rabbitmq
from learn.errors import TransientFailure
from learn.handlers.base import EventHandler
from learn.jev import client as jev
from learn.jev import session as jev_session


class TurnCompleteHandler(EventHandler):
    KIND = "turn.complete"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        last_assistant_message = payload.get("last_assistant_message", "")
        objective, steps = jev_session.peek(session_id)
        try:
            decision = jev.review_completion(objective, steps, last_assistant_message)
            # Publish the verdict as its own message instead of acting on it here -
            # JudgeResultHandler picks it up and decides what happens next (e.g.
            # whether field_pairs is worth processing).
            rabbitmq.publish(session_id, "judge.result", "jev_review_completion", {"decision": decision})
        except TransientFailure:
            # Leave jev_session in place - a message requeue (consumer.py's own
            # backoff) will retry with full context instead of an empty one. Re-calling
            # JEV on retry is harmless besides cost.
            raise
        except Exception:
            jev_session.clear(session_id)
            raise

        jev_session.clear(session_id)
        print(f"learn: jev decision for session={session_id}: {decision}")
