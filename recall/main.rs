use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{ChromeToolInput, PreToolUseHookSpecificOutput, PreToolUseRequest, PreToolUseResponse};

fn main() -> ExitCode {
    let mut raw = String::new();
    if let Err(err) = io::stdin().read_to_string(&mut raw) {
        eprintln!("recall: failed to read stdin: {err}");
        return ExitCode::SUCCESS;
    }

    let request: PreToolUseRequest = match serde_json::from_str(&raw) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("recall: failed to parse PreToolUseRequest: {err}");
            return ExitCode::SUCCESS;
        }
    };

    let tool_input = match ChromeToolInput::from_tool_call(&request.tool_name, request.tool_input.clone()) {
        Ok(tool_input) => tool_input,
        Err(err) => {
            eprintln!(
                "recall: failed to parse tool_input for {}: {err}",
                request.tool_name
            );
            ChromeToolInput::Unknown(request.tool_input.clone())
        }
    };

    let hint = shared::find_hint(&request.tool_name, &tool_input);

    let response = PreToolUseResponse {
        hook_specific_output: hint.map(|additional_context| PreToolUseHookSpecificOutput {
            hook_event_name: "PreToolUse".to_string(),
            additional_context: Some(additional_context),
            ..Default::default()
        }),
        ..Default::default()
    };

    if let Err(err) = write_response(&response) {
        eprintln!("recall: failed to write response: {err}");
    }

    ExitCode::SUCCESS
}

fn write_response(response: &PreToolUseResponse) -> serde_json::Result<()> {
    let body = serde_json::to_string(response)?;
    let _ = io::stdout().write_all(body.as_bytes());
    Ok(())
}
