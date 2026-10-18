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
            let parsed = ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone());

            // Send what's going into the tool call. No tool_response exists yet at
            // this point (the tool hasn't run), so page.snapshot can't apply here -
            // that stays exclusively capture's/PostToolUse's thing. Anything that
            // would be page.snapshot after the fact is sent as a plain "steps"
            // message now instead, since the only honest claim at this moment is
            // "this is about to happen," not "here is the page."
            if let Ok(tool_input) = &parsed {
                if shared::rabbitmq::message_kind(tool_input).is_some() {
                    let payload = shared::rabbitmq::marker_payload(tool_input)
                        .unwrap_or_else(|| serde_json::json!({ "tool_input": tool_input }));
                    rabbitmq::publish_blocking(
                        &request.session_id,
                        &request.tool_name,
                        shared::rabbitmq::RabbitMessageKind::Steps,
                        &payload,
                    );
                }
            }

            let hint = match &parsed {
                Ok(tool_input) => hint::find_hint(&request.tool_name, tool_input),
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

            // Cache every prompt regardless of whether it touches claude-in-chrome -
            // this is just a rolling recent-message window, independent of rabbitmq
            // routing below.
            redis_store::save_prompt(&request.common.session_id, &prompt);

            // Check first whether this prompt will even touch claude-in-chrome. Only
            // if it will do we publish it and look up a hint - no point queuing or
            // recall-ing for a prompt that was never going to invoke the browser.
            let hint = if hint::find_tool_predicate(&prompt) {
                rabbitmq::publish_blocking(
                    &request.common.session_id,
                    "user_prompt",
                    shared::rabbitmq::RabbitMessageKind::Prompt,
                    &serde_json::json!({ "prompt": prompt }),
                );

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
