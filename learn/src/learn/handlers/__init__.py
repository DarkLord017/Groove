from learn.handlers.base import EventHandler
from learn.handlers.page_snapshot import PageSnapshotHandler
from learn.handlers.prompt import PromptHandler
from learn.handlers.steps import StepsHandler
from learn.handlers.turn_complete import TurnCompleteHandler

_HANDLER_CLASSES: list[type[EventHandler]] = [
    PageSnapshotHandler,
    StepsHandler,
    PromptHandler,
    TurnCompleteHandler,
]

HANDLERS: dict[str, EventHandler] = {cls.KIND: cls() for cls in _HANDLER_CLASSES}

__all__ = ["EventHandler", "HANDLERS"]
