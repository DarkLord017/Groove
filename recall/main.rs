use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{
    AdditionalContextOutput, ChromeToolInput, HookEvent, PostToolUseHookSpecificOutput,
    PostToolUseResponse, PreToolUseHookSpecificOutput, PreToolUseResponse, UserPromptSubmitResponse,
};

mod browser_predicate;
mod condition_match;
mod domain;
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
                Ok(tool_input) => hint::find_hint(&request.session_id, tool_input),
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

                hint::find_prompt_hint(&prompt)
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

        // The `ReadPage -> stage` leg: only read_page/find/get_page_text hand back real
        // page text to check stages against - every other claude-in-chrome tool (clicks,
        // typing, etc.) returns nothing useful here, same constraint covered in hint.rs's
        // docs. browser_batch-wrapped calls aren't unwrapped here (unlike capture's own
        // PostToolUse handling) - out of scope for this pass, standalone calls only.
        HookEvent::PostToolUse(request) => {
            let tab_id_and_text = ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone())
                .ok()
                .and_then(|tool_input| match tool_input {
                    ChromeToolInput::ReadPage(input) => Some(input.tab_id),
                    ChromeToolInput::Find(input) => Some(input.tab_id),
                    ChromeToolInput::GetPageText(input) => Some(input.tab_id),
                    _ => None,
                })
                .zip(extract_page_text(&request.tool_response));

            let hint = tab_id_and_text
                .and_then(|(tab_id, text)| hint::find_stage_hint(&request.session_id, tab_id, &text))
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

        // Turn complete - reset this session's run state (cached intent/domain, done
        // stages) so the next run starts clean rather than inheriting this one's.
        HookEvent::Stop(request) => {
            redis_store::clear_run_state(&request.common.session_id);
            "{}".to_string()
        }

        _ => "{}".to_string(),
    };

    io::stdout().write_all(response_body.as_bytes())?;
    Ok(())
}

// read_page/find/get_page_text's tool_response is either a plain string or (inside a
// batch, or depending on the client) an array of {"text": ..., "type": "text"} blocks -
// same ambiguity capture's own cache_elements handles, duplicated here in miniature
// rather than shared across the two binaries for a single small helper.
fn extract_page_text(tool_response: &serde_json::Value) -> Option<String> {
    match tool_response {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => {
            let joined: String = blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}
