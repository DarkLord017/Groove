use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{ChromeToolInput, PostToolUseRequest, PostToolUseResponse};

fn main() -> ExitCode {
    let mut raw = String::new();
    if let Err(err) = io::stdin().read_to_string(&mut raw) {
        eprintln!("capture: failed to read stdin: {err}");
        return ExitCode::SUCCESS;
    }

    let request: PostToolUseRequest = match serde_json::from_str(&raw) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("capture: failed to parse PostToolUseRequest: {err}");
            return ExitCode::SUCCESS;
        }
    };

    let tool_input = match ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone()) {
        Ok(tool_input) => tool_input,
        Err(err) => {
            eprintln!(
                "capture: failed to parse tool_input for {}: {err}",
                request.tool_name
            );
            ChromeToolInput::Unknown(request.tool_input.clone())
        }
    };

    record_step(&request, &tool_input);

    let response = PostToolUseResponse::default();
    if let Err(err) = write_response(&response) {
        eprintln!("capture: failed to write response: {err}");
    }

    ExitCode::SUCCESS
}

// TODO: persist the step once the capture store (db/log) exists. For now this just
// confirms the hook is wired end-to-end.
fn record_step(request: &PostToolUseRequest, tool_input: &ChromeToolInput) {
    eprintln!(
        "capture: session={} tool={} input={:?}",
        request.session_id, request.tool_name, tool_input
    );
}

fn write_response(response: &PostToolUseResponse) -> serde_json::Result<()> {
    let body = serde_json::to_string(response)?;
    let _ = io::stdout().write_all(body.as_bytes());
    Ok(())
}
