// Stand-in for publishing to RabbitMQ. Logs exactly what it would publish.
// TODO: replace with a real publish to the "steps" topic exchange once RabbitMQ exists.

use shared::PostToolUseRequest;

pub fn forward_step(request: &PostToolUseRequest) {
    eprintln!(
        "capture: [rabbitmq stub] publish steps session={} tool={} tool_input={} tool_response={}",
        request.session_id, request.tool_name, request.tool_input, request.tool_response
    );
}
