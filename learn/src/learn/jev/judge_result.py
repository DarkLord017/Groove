from learn import facts, field_pairs, tasks
from learn.awm import workflow as awm
from learn.handlers.base import EventHandler
from learn.hmt import site_tree
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

                domain = payload.get("domain") or "unknown"
                objective = payload.get("objective") or ""

                site_tree.record_run(domain, objective, pairs)

                # Only bother inducing (an LLM call) once this domain has actually
                # been visited frequently enough recently - see awm.should_induce.
                if awm.should_induce(domain):
                    induced = awm.induce_workflow(objective, pairs)
                    if induced:
                        awm.record_workflow(domain, induced)

        field_pairs.clear(session_id)
        site_tree.clear_domain(session_id)
