from learn import field_pairs
from learn.handlers.base import EventHandler
from learn.hmt import site_tree
from learn.jev import session as jev_session


class StepsHandler(EventHandler):
    """Handles both "steps" and "page.snapshot" (registered for both in
    handlers/__init__.py) - a page snapshot gets the exact same treatment as any other
    step: logged to the JEV work accumulator, and checked for a click/type field pair.
    field_pairs.observe_step is a no-op for anything that isn't a Computer action, so
    running it on page.snapshot payloads (ReadPage/Find) is harmless.
    """

    KIND = "steps"

    def handle(self, session_id: str, label: str, payload: dict) -> None:
        # Every step is published twice - once by recall on PreToolUse (tool hasn't
        # run yet, no tool_response), once by capture on PostToolUse (the real
        # tool_response). Only the second copy is worth logging to JEV's work list;
        # the first would otherwise waste a slot dumping the raw, incomplete payload.
        if payload.get("tool_response") is not None:
            jev_session.append_work(session_id, jev_session.summarize_step(label, payload))
            navigate = payload.get("tool_input", {}).get("Navigate")
            if navigate and navigate.get("url"):
                site_tree.set_domain(session_id, navigate["url"])
        field_pairs.observe_step(session_id, payload)
