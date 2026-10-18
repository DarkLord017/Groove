from abc import ABC, abstractmethod


class EventHandler(ABC):
    """Interface every per-kind handler implements. KIND is the dispatch key the
    consumer looks up in HANDLERS - it must match one of the routing_key() strings
    produced by shared::rabbitmq::RabbitMessageKind on the Rust side.
    """

    KIND: str

    @abstractmethod
    def handle(self, session_id: str, label: str, payload: dict) -> None:
        """Process one message. Raise TransientFailure (from learn.errors) if a
        dependency is unreachable; any other exception is treated as unfixable and the
        message goes to the DLQ.
        """
