use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{ChromeToolInput, HookEvent, PreToolUseHookSpecificOutput, PreToolUseResponse , UserPromptSubmitResponse , AdditionalContextOutput};

mod browser_predicate;
mod error;
mod hint;

use error::RecallError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("recall: {err}");
            ExitCode::SUCCESS
        }
    }
}

fn run() -> Result<(), RecallError> {
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

    let response_body = match &event {
        HookEvent::PreToolUse(request) => {
            let hint = match ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone()) {
                Ok(tool_input) => hint::find_hint(&request.tool_name, &tool_input),
                Err(err) => {
                    eprintln!(
                        "recall: schema mismatch - {} is a known claude-in-chrome tool but its tool_input didn't deserialize: {err}",
                        request.tool_name
                    );
                    None
                }
            }
            .map(|mut text| {
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

        HookEvent::UserPromptSubmit(request) => {
            let prompt = request.prompt.clone();
            let hint = if hint::find_tool_predicate(&prompt) {
                hint::find_hint(&request.common.hook_event_name, &prompt)
            } else {
                None
            }
            .map(|mut text| {
                let violations = shared::privacy::redact_str(&mut text);
                if !violations.is_empty() {
                    eprintln!(
                        "recall: redacted {} sensitive value(s) from outgoing hint",
                        violations.len()
                    );
                }
                text
            });

            let response = UserPromptSubmitResponse {
                hook_specific_output: hint.map(|additional_context| {
                    AdditionalContextOutput {
                        hook_event_name: "UserPromptSubmit".to_string(),
                        additional_context: Some(additional_context),
                        ..Default::default()
                    }
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
