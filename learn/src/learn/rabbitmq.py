"""Publishes messages from learn back onto RabbitMQ - the Python-side counterpart to
Rust's rabbitmq::publish_blocking (used by capture/recall). Lets one handler's result
(e.g. JEV's verdict) become its own message for a later handler to pick up, instead of
cramming every downstream step into a single handler call.

Reuses the "turn.complete" routing key, which is already bound to every worker.N queue
(see integrations/rabbitmq/setup.rs) - no new RabbitMQ binding needed. The routing key
only controls which physical queue a message lands on; learn's own dispatch
(consumer.py) decides what to do with a message purely from its "kind" field in the
body, which is independent of the routing key that delivered it.
"""
import json
import os

import pika

from learn import session_queue
from learn.errors import TransientFailure

RABBITMQ_URL = os.environ.get("RABBITMQ_URL", "amqp://guest:guest@127.0.0.1:5672/%2f")
EXCHANGE = os.environ.get("RABBITMQ_EXCHANGE", "groove.events")

_connection = None
_channel = None

def _channel_ref():
    global _connection, _channel
    if _channel is None or _channel.is_closed:
        _connection = pika.BlockingConnection(pika.URLParameters(RABBITMQ_URL))
        _channel = _connection.channel()
    return _channel


def publish(session_id: str, kind: str, label: str, payload: dict) -> None:
    queue_num = session_queue.get(session_id)
    if queue_num is None:
        raise TransientFailure(f"rabbitmq publish: no known queue for session={session_id}")

    body = json.dumps(
        {
            "session_id": session_id,
            "label": label,
            "kind": kind,
            "queue": queue_num,
            "payload": payload,
        }
    ).encode()

    try:
        channel = _channel_ref()
        channel.basic_publish(
            exchange=EXCHANGE,
            routing_key=f"turn.complete.{queue_num}",
            body=body,
            properties=pika.BasicProperties(content_type="application/json", delivery_mode=2),
        )
    except pika.exceptions.AMQPError as e:
        raise TransientFailure(f"rabbitmq publish failed for kind={kind}: {e}") from e
