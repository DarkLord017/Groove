use shared::rabbitmq::queue_for_session;

fn main() {
    let session_id = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: rabbitmq-queue-for <session_id>");
        std::process::exit(1);
    });
    println!("{}", queue_for_session(&session_id));
}
