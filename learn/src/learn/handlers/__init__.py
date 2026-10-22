from learn.handlers.base import EventHandler
from learn.handlers.prompt import PromptHandler
from learn.handlers.steps import StepsHandler
from learn.jev.judge_result import JudgeResultHandler
from learn.jev.turn_complete import TurnCompleteHandler

_HANDLER_CLASSES: list[type[EventHandler]] = [
    StepsHandler,
    PromptHandler,
    TurnCompleteHandler,
    JudgeResultHandler,
]

HANDLERS: dict[str, EventHandler] = {cls.KIND: cls() for cls in _HANDLER_CLASSES}

# page.snapshot (ReadPage/Find) gets identical treatment to steps - see StepsHandler's
# docstring. No separate handler class needed for it.
HANDLERS["page.snapshot"] = HANDLERS["steps"]

__all__ = ["EventHandler", "HANDLERS"]
