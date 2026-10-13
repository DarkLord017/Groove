use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{ChromeToolInput, HookEvent, PostToolUseHookSpecificOutput, PostToolUseResponse};

mod error;
mod normalizer;
mod privacy;

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

    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let event = HookEvent::from_value(value)?;

    let mut normalized = normalizer::normalize(&event);
    let violations = privacy::redact(&mut normalized.payload);
    if !violations.is_empty() {
        eprintln!("capture: {}", CaptureError::privacy(violations));
    }

    record_step(&normalized);

    let response_body = match &event {
        HookEvent::PostToolUse(request) => {
            let tool_input = ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone())
                .unwrap_or_else(|_| ChromeToolInput::Unknown(request.tool_input.clone()));
            let hint = shared::find_hint(&request.tool_name, &tool_input);
            let response = PostToolUseResponse {
                hook_specific_output: hint.map(|additional_context| PostToolUseHookSpecificOutput {
                    hook_event_name: "PostToolUse".to_string(),
                    additional_context: Some(additional_context),
                    ..Default::default()
                }),
                ..Default::default()
            };
            serde_json::to_string(&response)?
        }
        _ => "{}".to_string(),
    };

    io::stdout().write_all(response_body.as_bytes())?;
    Ok(())
}

// TODO: persist the normalized step once the capture store (db/log) exists.
fn record_step(event: &normalizer::NormalizedEvent) {
    eprintln!(
        "capture: session={} event={} tool={:?} payload={}",
        event.session_id, event.hook_event_name, event.tool_name, event.payload
    );
}
