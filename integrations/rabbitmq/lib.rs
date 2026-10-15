use std::time::Duration;

use lapin::options::BasicPublishOptions;
use lapin::{BasicProperties, Connection, ConnectionProperties};
use shared::rabbitmq::{queue_for_session, RabbitMessageKind};
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(1000);

fn connection_url() -> String {
    std::env::var("RABBITMQ_URL").unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".to_string())
}

fn exchange_name() -> String {
    std::env::var("RABBITMQ_EXCHANGE").unwrap_or_else(|_| "groove.events".to_string())
}

/// Synchronous entry point for capture/recall, which are themselves plain synchronous
/// binaries - this is the only place async shows up, contained to a throwaway runtime
/// that exists just for the duration of one publish.
pub fn publish_blocking(session_id: &str, label: &str, kind: RabbitMessageKind, payload: &serde_json::Value) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("rabbitmq: failed to start runtime: {err}");
            return;
        }
    };
    runtime.block_on(publish(session_id, label, kind, payload));
}

async fn publish(session_id: &str, label: &str, kind: RabbitMessageKind, payload: &serde_json::Value) {
    let queue_num = queue_for_session(session_id);
    let routing_key = format!("{}.{queue_num}", kind.routing_key());

    let body = serde_json::json!({
        "session_id": session_id,
        "label": label,
        "kind": kind.routing_key(),
        "queue": queue_num,
        "payload": payload,
    });
    let bytes = match serde_json::to_vec(&body) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("rabbitmq: failed to serialize message for {routing_key}: {err}");
            return;
        }
    };

    let exchange = exchange_name();
    let uri = connection_url();

    let result = timeout(CONNECT_TIMEOUT, async {
        let conn = Connection::connect(&uri, ConnectionProperties::default()).await?;
        let channel = conn.create_channel().await?;
        channel
            .basic_publish(
                &exchange,
                &routing_key,
                BasicPublishOptions::default(),
                &bytes,
                BasicProperties::default()
                    .with_content_type("application/json".into())
                    .with_delivery_mode(2), // persistent
            )
            .await?
            .await?;
        conn.close(200, "done").await?;
        Ok::<(), lapin::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            eprintln!("rabbitmq: published {exchange}/{routing_key} (session={session_id}): {label}");
        }
        Ok(Err(err)) => {
            eprintln!("rabbitmq: publish failed for {exchange}/{routing_key}: {err}");
        }
        Err(_) => {
            eprintln!(
                "rabbitmq: publish to {exchange}/{routing_key} timed out after {CONNECT_TIMEOUT:?}"
            );
        }
    }
}
