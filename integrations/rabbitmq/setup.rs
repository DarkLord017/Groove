
use lapin::options::{ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions};
use lapin::types::{AMQPValue, FieldTable};
use lapin::{Connection, ConnectionProperties, ExchangeKind};
use shared::rabbitmq::{RabbitMessageKind, QUEUE_COUNT};

const DLX_EXCHANGE: &str = "groove.dlx";
const DLQ_QUEUE: &str = "groove.dlq";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let uri = std::env::var("RABBITMQ_URL")
        .unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".to_string());
    let exchange = std::env::var("RABBITMQ_EXCHANGE").unwrap_or_else(|_| "groove.events".to_string());

    let conn = Connection::connect(&uri, ConnectionProperties::default())
        .await
        .expect("connect to rabbitmq (check RABBITMQ_URL)");
    let channel = conn.create_channel().await.expect("create channel");

    channel
        .exchange_declare(
            &exchange,
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare exchange");

    // Dead-letter side: a fanout exchange with one queue behind it. Any message nacked
    // with requeue=false from a worker queue (bad data, not a transient failure) lands
    // here for manual inspection/replay - see learn/ for the consumer-side half of this.
    channel
        .exchange_declare(
            DLX_EXCHANGE,
            ExchangeKind::Fanout,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare dead-letter exchange");

    channel
        .queue_declare(
            DLQ_QUEUE,
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare dead-letter queue");

    channel
        .queue_bind(
            DLQ_QUEUE,
            DLX_EXCHANGE,
            "",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("bind dead-letter queue");

    let kinds = [
        RabbitMessageKind::PageSnapshot,
        RabbitMessageKind::Steps,
        RabbitMessageKind::Prompt,
        RabbitMessageKind::TurnComplete,
    ];

    let mut worker_queue_args = FieldTable::default();
    worker_queue_args.insert(
        "x-dead-letter-exchange".into(),
        AMQPValue::LongString(DLX_EXCHANGE.into()),
    );

    for queue_num in 0..QUEUE_COUNT {
        let queue_name = format!("worker.{queue_num}");
        channel
            .queue_declare(
                &queue_name,
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                worker_queue_args.clone(),
            )
            .await
            .unwrap_or_else(|err| panic!("declare queue {queue_name}: {err}"));

        for kind in kinds {
            let routing_key = format!("{}.{queue_num}", kind.routing_key());
            channel
                .queue_bind(
                    &queue_name,
                    &exchange,
                    &routing_key,
                    QueueBindOptions::default(),
                    FieldTable::default(),
                )
                .await
                .unwrap_or_else(|err| panic!("bind {queue_name} to {routing_key}: {err}"));
        }
    }

    println!(
        "rabbitmq-setup: declared exchange '{exchange}' (topic) and {QUEUE_COUNT} queues (worker.0..worker.{}), each bound to page.snapshot/steps/prompt/turn.complete for its own queue number, each dead-lettering to '{DLX_EXCHANGE}' -> '{DLQ_QUEUE}' on nack(requeue=false)",
        QUEUE_COUNT - 1
    );
}
