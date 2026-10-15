
use lapin::options::{ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions};
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties, ExchangeKind};
use shared::rabbitmq::{RabbitMessageKind, QUEUE_COUNT};

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

    let kinds = [
        RabbitMessageKind::PageSnapshot,
        RabbitMessageKind::Steps,
        RabbitMessageKind::Prompt,
    ];

    for queue_num in 0..QUEUE_COUNT {
        let queue_name = format!("worker.{queue_num}");
        channel
            .queue_declare(
                &queue_name,
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                FieldTable::default(),
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
        "rabbitmq-setup: declared exchange '{exchange}' (topic) and {QUEUE_COUNT} queues (worker.0..worker.{}), each bound to page.snapshot/steps/prompt for its own queue number",
        QUEUE_COUNT - 1
    );
}
