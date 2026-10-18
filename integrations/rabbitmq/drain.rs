use std::io::{self, Write};
 use std::time::Duration;

use lapin::options::{BasicAckOptions, BasicGetOptions};
use lapin::{Connection, ConnectionProperties};
use tokio::time::{sleep, timeout};

const CONNECT_TIMEOUT: Duration = Duration::from_millis(2000);
const OVERALL_TIMEOUT: Duration = Duration::from_secs(10);
const EMPTY_STREAK_LIMIT: u32 = 3;
const POLL_DELAY: Duration = Duration::from_millis(300);

fn connection_url() -> String {
    std::env::var("RABBITMQ_URL").unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".to_string())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let queue = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: rabbitmq-drain <queue-name>");
        std::process::exit(1);
    });

    let result = timeout(OVERALL_TIMEOUT, drain(&queue)).await;
    match result {
        Ok(Ok(count)) => {
            eprintln!("rabbitmq-drain: read {count} message(s) from {queue}");
        }
        Ok(Err(err)) => {
            eprintln!("rabbitmq-drain: error draining {queue}: {err}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("rabbitmq-drain: timed out after {OVERALL_TIMEOUT:?} draining {queue}");
            std::process::exit(1);
        }
    }
}

async fn drain(queue: &str) -> Result<usize, lapin::Error> {
    let conn = timeout(CONNECT_TIMEOUT, Connection::connect(&connection_url(), ConnectionProperties::default()))
        .await
        .expect("connect timed out")?;
    let channel = conn.create_channel().await?;

    let mut count = 0;
    let mut empty_streak = 0;

    while empty_streak < EMPTY_STREAK_LIMIT {
        match channel.basic_get(queue, BasicGetOptions::default()).await? {
            Some(delivery) => {
                empty_streak = 0;
                io::stdout().write_all(&delivery.data).ok();
                io::stdout().write_all(b"\n").ok();
                delivery.ack(BasicAckOptions::default()).await?;
                count += 1;
            }
            None => {
                empty_streak += 1;
                sleep(POLL_DELAY).await;
            }
        }
    }

    conn.close(200, "done").await?;
    Ok(count)
}
