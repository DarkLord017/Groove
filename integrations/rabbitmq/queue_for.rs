// Prints which of the fixed pool of queues a session_id routes to. No network call -
// queue_for_session is a pure function - this just exposes it to bash eval scripts that
// need to know which queue to drain for a given session, without reimplementing the
// hash in a second language.
//
// Usage: rabbitmq-queue-for <session_id>

use shared::rabbitmq::queue_for_session;

fn main() {
    let session_id = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: rabbitmq-queue-for <session_id>");
        std::process::exit(1);
    });
    println!("{}", queue_for_session(&session_id));
}
