from learn import field_pairs
from learn.handlers.base import EventHandler
from learn.jev import client as jev


class JudgeResultHandler(EventHandler):
    KIND = "judge.result"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        decision = payload.get("decision", {})

        if jev.indicates_success(decision):
            pairs = field_pairs.peek(session_id)
            if pairs:
                print(f"learn: field pairs for session={session_id}: {pairs}")
                # Linking these pairs back to the user's own prompt wording (e.g.
                # "home" -> "Destination" -> "Roorkee") needs an LLM judgment call or a
                # timing-based heuristic - GLiNER can't do it (benchmarked: it only
                # tags real named entities, never generic/anaphoric words like "home").
                # Skipped for v1 - the pairs themselves are still useful without it.

        field_pairs.clear(session_id)
