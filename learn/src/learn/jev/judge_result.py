from learn import facts, field_pairs, tasks
from learn.handlers.base import EventHandler
from learn.jev import client as jev


class JudgeResultHandler(EventHandler):
    KIND = "judge.result"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        decision = payload.get("decision", {})

        if jev.indicates_success(decision):
            pairs = field_pairs.peek(session_id)
            if pairs:
                classification = jev.classify_pairs(pairs)
                for i, pair in enumerate(pairs):
                    kind = jev.pair_classification(classification, i)
                    if kind == "personal":
                        facts.save(pair["key"], pair["value"])
                    elif kind == "task":
                        tasks.observe(f"{pair['key']}: {pair['value']}")
                    # "not_needed", or a missing/malformed classification -> skip.

        field_pairs.clear(session_id)
