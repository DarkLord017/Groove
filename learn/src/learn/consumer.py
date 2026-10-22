import json
import os

import pika

from learn import session_queue
from learn.errors import TransientFailure
from learn.handlers import HANDLERS

RABBITMQ_URL = os.environ.get("RABBITMQ_URL", "amqp://guest:guest@127.0.0.1:5672/%2f")
QUEUE_COUNT = 32
TRANSIENT_BACKOFF_SECONDS = 5


def handle_message(channel, method, _properties, body) -> None:
    try:
        msg = json.loads(body)
    except json.JSONDecodeError:
        print("learn: malformed JSON, sending to DLQ")
        channel.basic_nack(delivery_tag=method.delivery_tag, requeue=False)
        return

    kind = msg.get("kind")
    handler = HANDLERS.get(kind)
    if handler is None:
        print(f"learn: no handler registered for kind={kind!r}, sending to DLQ")
        channel.basic_nack(delivery_tag=method.delivery_tag, requeue=False)
        return

    session_id = msg.get("session_id")
    session_queue.remember(session_id, msg.get("queue"))
    try:
        handler.handle(session_id, msg.get("label", ""), msg.get("payload", {}))
        channel.basic_ack(delivery_tag=method.delivery_tag)
    except TransientFailure as e:
        print(f"learn: transient failure (session={session_id}, kind={kind}): {e} - requeueing and backing off")
        channel.basic_nack(delivery_tag=method.delivery_tag, requeue=True)
        channel.connection.sleep(TRANSIENT_BACKOFF_SECONDS)
    except Exception as e:
        print(f"learn: permanent failure (session={session_id}, kind={kind}): {e} - sending to DLQ")
        channel.basic_nack(delivery_tag=method.delivery_tag, requeue=False)


def main() -> None:
    connection = pika.BlockingConnection(pika.URLParameters(RABBITMQ_URL))
    channel = connection.channel()
    channel.basic_qos(prefetch_count=10)

    for n in range(QUEUE_COUNT):
        channel.basic_consume(queue=f"worker.{n}", on_message_callback=handle_message)

    print(f"learn: consuming worker.0..worker.{QUEUE_COUNT - 1}")
    try:
        channel.start_consuming()
    except KeyboardInterrupt:
        channel.stop_consuming()
    finally:
        connection.close()


if __name__ == "__main__":
    main()
