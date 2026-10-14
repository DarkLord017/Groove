use std::io::{self, Read, Write};
use std::process::ExitCode;

use capture::CaptureError;
use shared::{ChromeToolInput, HookEvent, PreToolUseHookSpecificOutput, PreToolUseResponse};

mod hint;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("recall: {err}");
            ExitCode::SUCCESS
        }
    }
}

fn run() -> Result<(), CaptureError> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;

    let mut value: serde_json::Value = serde_json::from_str(&raw)?;

    // Redact at the boundary, before anything derives from the payload: the HookEvent,
    // the normalized record, and the tool_input handed to find_hint all come from this
    // value, so none of them can observe a secret.
    let violations = shared::privacy::redact(&mut value);
    if !violations.is_empty() {
        let joined: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
        eprintln!(
            "recall: redacted {} sensitive value(s): {}",
            violations.len(),
            joined.join(", ")
        );
    }

    let event = HookEvent::from_value(value)?;

    let normalized = capture::normalize(&event);
    capture::record_step(&normalized);

    let response_body = match &event {
        HookEvent::PreToolUse(request) => {
            let tool_input = ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone())
                .unwrap_or_else(|_| ChromeToolInput::Unknown(request.tool_input.clone()));
            let hint = hint::find_hint(&request.tool_name, &tool_input).map(|mut text| {
                // Redact on the way out too: once find_hint reads from real storage,
                // this is what stops a secret captured in an earlier session from
                // round-tripping straight back into Claude's context here.
                let violations = shared::privacy::redact_str(&mut text);
                if !violations.is_empty() {
                    eprintln!(
                        "recall: redacted {} sensitive value(s) from outgoing hint",
                        violations.len()
                    );
                }
                text
            });
            let response = PreToolUseResponse {
                hook_specific_output: hint.map(|additional_context| PreToolUseHookSpecificOutput {
                    hook_event_name: "PreToolUse".to_string(),
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
