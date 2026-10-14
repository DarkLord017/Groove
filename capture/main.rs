use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::PostToolUseRequest;

mod error;

use error::CaptureError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("capture: {err}");
            ExitCode::SUCCESS
        }
    }
}

fn run() -> Result<(), CaptureError> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;

    let mut value: serde_json::Value = serde_json::from_str(&raw)?;

    // Redact at the boundary, before anything derives from the payload.
    let violations = shared::privacy::redact(&mut value);
    if !violations.is_empty() {
        let joined: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
        eprintln!(
            "capture: redacted {} sensitive value(s): {}",
            violations.len(),
            joined.join(", ")
        );
    }

    let request: PostToolUseRequest = serde_json::from_value(value)?;

    eprintln!(
        "capture: session={} tool={} tool_input={} tool_response={}",
        request.session_id, request.tool_name, request.tool_input, request.tool_response
    );

    io::stdout().write_all(b"{}")?;
    Ok(())
}
