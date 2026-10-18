use std::io::{self, Read, Write};
use std::process::ExitCode;

use shared::{
    BrowserBatchInput, ChromeToolInput, ComputerAction, ComputerInput, HookEvent,
    PostToolUseRequest, SessionEndRequest, StopRequest,
};

mod elements;
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

    let event = HookEvent::from_value(value)?;

    match &event {
        HookEvent::PostToolUse(request) => handle_post_tool_use(request),
        HookEvent::SessionEnd(request) => handle_session_end(request),
        HookEvent::Stop(request) => handle_stop(request),
        _ => {}
    }

    io::stdout().write_all(b"{}")?;
    Ok(())
}

fn handle_post_tool_use(request: &PostToolUseRequest) {
    eprintln!(
        "capture: session={} tool={} tool_input={} tool_response={}",
        request.session_id, request.tool_name, request.tool_input, request.tool_response
    );

    handle_tool_call(
        &request.session_id,
        &request.tool_name,
        request.tool_input.clone(),
        Some(&request.tool_response),
        None,
    );
}

fn handle_session_end(request: &SessionEndRequest) {
    eprintln!(
        "capture: session_end session={} reason={}",
        request.common.session_id, request.reason
    );
    redis_store::delete_session_maps(&request.common.session_id);
}

fn handle_stop(request: &StopRequest) {
    eprintln!(
        "capture: stop session={} (turn complete)",
        request.common.session_id
    );
    rabbitmq::publish_blocking(
        &request.common.session_id,
        "stop",
        shared::rabbitmq::RabbitMessageKind::TurnComplete,
        &serde_json::json!({ "last_assistant_message": request.last_assistant_message }),
    );
}

// Shared by the top-level PostToolUse dispatch and by browser_batch's inner actions -
// same map rules either way, just without a separable tool_response for inner actions
// (see handle_browser_batch). forced_rabbitmq_kind overrides the normal per-tool
// classification: browser_batch's inner actions always publish "steps", regardless of
// what kind that tool would normally get standalone (read_page/find included).
fn handle_tool_call(
    session_id: &str,
    tool_name: &str,
    tool_input: serde_json::Value,
    tool_response: Option<&serde_json::Value>,
    forced_rabbitmq_kind: Option<shared::rabbitmq::RabbitMessageKind>,
) {
    let parsed = ChromeToolInput::from_tool_call(tool_name, tool_input);

    match &parsed {
        // Write: read_page/find populate the map.
        Ok(ChromeToolInput::ReadPage(input)) => {
            cache_elements(session_id, input.tab_id, tool_response, tool_name)
        }
        Ok(ChromeToolInput::Find(input)) => {
            cache_elements(session_id, input.tab_id, tool_response, tool_name)
        }

        // Read: these consult the map.
        Ok(ChromeToolInput::Computer(input)) => resolve_computer(session_id, input),
        Ok(ChromeToolInput::FormInput(input)) => resolve_ref(session_id, input.tab_id, &input.r#ref),
        Ok(ChromeToolInput::FileUpload(input)) => resolve_ref(session_id, input.tab_id, &input.r#ref),
        Ok(ChromeToolInput::UploadImage(input)) => match &input.r#ref {
            Some(ref_id) => resolve_ref(session_id, input.tab_id, ref_id),
            None => eprintln!("capture: [lookup] upload_image has no ref - unknown"),
        },

        // Delete: these invalidate the map. A new page means old refs are dead; closing
        // a tab means its map is dead; switching browsers means every tab in the
        // session's map is dead (different browser, different tabs entirely).
        Ok(ChromeToolInput::Navigate(input)) => match input.tab_id {
            Some(tab_id) => redis_store::delete_tab_maps(session_id, tab_id),
            None => eprintln!(
                "capture: [delete] navigate with no explicit tabId - cannot determine which tab's map to delete"
            ),
        },
        Ok(ChromeToolInput::TabsClose(input)) => {
            redis_store::delete_tab_maps(session_id, input.tab_id as f64)
        }
        Ok(ChromeToolInput::SelectBrowser(_)) | Ok(ChromeToolInput::SwitchBrowser(_)) => {
            redis_store::delete_session_maps(session_id)
        }

        // Split: run every inner action through these same rules, in order. The batch
        // publishes its own "steps" message per inner action, so the outer
        // browser_batch call publishes nothing itself (it isn't in the allow-list
        // anyway - see shared::rabbitmq::message_kind).
        Ok(ChromeToolInput::BrowserBatch(input)) => {
            handle_browser_batch(session_id, input, tool_response);
            return;
        }

        // No normalization: TabsContext, TabsCreate (a new tab starts with an empty
        // map), GetPageText (text only, no refs), ResizeWindow (refs stay valid, only
        // positions change), JavascriptTool, GifCreator, ReadConsoleMessages,
        // ReadNetworkRequests, ShortcutsExecute, ShortcutsList,
        // ListConnectedBrowsers, and anything not a claude-in-chrome tool at all -
        // none of these touch the map, on purpose.
        Ok(_) => {}

        Err(err) => {
            eprintln!(
                "capture: schema mismatch - {tool_name} is a known claude-in-chrome tool but its tool_input didn't deserialize: {err}"
            );
        }
    }

    if let Ok(mut chrome_tool_input) = parsed {
        let kind = forced_rabbitmq_kind.or_else(|| shared::rabbitmq::message_kind(&chrome_tool_input));
        if let Some(kind) = kind {
            resolve_ref_for_payload(session_id, &mut chrome_tool_input);
            let payload = shared::rabbitmq::marker_payload(&chrome_tool_input).unwrap_or_else(|| {
                serde_json::json!({
                    "tool_input": chrome_tool_input,
                    "tool_response": tool_response,
                })
            });
            rabbitmq::publish_blocking(session_id, tool_name, kind, &payload);
        }
    }
}

// Replaces a ref field's raw ref_id with the human-readable description an earlier
// read_page/find cached for it (via Redis), right before publish - "ref_1" means
// nothing to anything reading this off RabbitMQ later, the description does. Leaves
// the raw ref_id in place if nothing is cached for it (fail open: something beats
// nothing, and a cache miss shouldn't block the publish).
fn resolve_ref_for_payload(session_id: &str, chrome_tool_input: &mut ChromeToolInput) {
    match chrome_tool_input {
        ChromeToolInput::Computer(input) => {
            if let Some(ref_id) = &input.r#ref {
                if let Some(description) = redis_store::lookup_element(session_id, input.tab_id, ref_id) {
                    input.r#ref = Some(description);
                }
            }
        }
        ChromeToolInput::FormInput(input) => {
            if let Some(description) = redis_store::lookup_element(session_id, input.tab_id, &input.r#ref) {
                input.r#ref = description;
            }
        }
        ChromeToolInput::FileUpload(input) => {
            if let Some(description) = redis_store::lookup_element(session_id, input.tab_id, &input.r#ref) {
                input.r#ref = description;
            }
        }
        ChromeToolInput::UploadImage(input) => {
            if let Some(ref_id) = &input.r#ref {
                if let Some(description) = redis_store::lookup_element(session_id, input.tab_id, ref_id) {
                    input.r#ref = Some(description);
                }
            }
        }
        _ => {}
    }
}

fn handle_browser_batch(
    session_id: &str,
    input: &BrowserBatchInput,
    tool_response: Option<&serde_json::Value>,
) {
    eprintln!(
        "capture: [batch] splitting {} inner action(s)",
        input.actions.len()
    );
    let mut blocks = tool_response.map(extract_batch_text_blocks).unwrap_or_default();

    for action in &input.actions {
        let tool_name = format!("mcp__claude-in-chrome__{}", action.name);
        let own_response = take_batch_action_response(&mut blocks, &action.name);
        handle_tool_call(
            session_id,
            &tool_name,
            action.input.clone(),
            own_response.as_ref(),
            Some(shared::rabbitmq::RabbitMessageKind::Steps),
        );
    }
}

fn extract_batch_text_blocks(tool_response: &serde_json::Value) -> Vec<String> {
    tool_response
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}


fn take_batch_action_response(
    blocks: &mut Vec<String>,
    action_name: &str,
) -> Option<serde_json::Value> {
    let prefix = format!("[{action_name}]");
    let index = blocks.iter().position(|text| text.starts_with(&prefix))?;
    let text = blocks.remove(index);
    let stripped = text.strip_prefix(&prefix).unwrap_or(&text).trim_start().to_string();
    Some(serde_json::Value::String(stripped))
}

fn cache_elements(
    session_id: &str,
    tab_id: f64,
    tool_response: Option<&serde_json::Value>,
    tool_name: &str,
) {
    let Some(tool_response) = tool_response else {
        eprintln!(
            "capture: [cache] {tool_name} inside a batch has no separable response - skipping cache"
        );
        return;
    };
    if let Some(text) = tool_response.as_str() {
        let found = elements::parse_elements(text);
        redis_store::cache_elements(session_id, Some(tab_id), &found);
    }
}

fn resolve_ref(session_id: &str, tab_id: f64, ref_id: &str) {
    match redis_store::lookup_element(session_id, tab_id, ref_id) {
        Some(description) => eprintln!("capture: [lookup] {ref_id} => {description}"),
        None => eprintln!("capture: [lookup] {ref_id} => (not cached)"),
    }
}

// computer carries one tool_input shape for several different actions. Only some of
// them reference an element at all, and only clicks set what a later type/key action
// (which carries no ref of its own) resolves against.
fn resolve_computer(session_id: &str, input: &ComputerInput) {
    let is_click = matches!(
        input.action,
        ComputerAction::LeftClick
            | ComputerAction::RightClick
            | ComputerAction::DoubleClick
            | ComputerAction::TripleClick
            | ComputerAction::LeftClickDrag
    );
    let is_ref_lookup_action =
        is_click || matches!(input.action, ComputerAction::Hover | ComputerAction::ScrollTo);
    let is_type_action = matches!(input.action, ComputerAction::Type | ComputerAction::Key);

    if let Some(ref_id) = &input.r#ref {
        if is_ref_lookup_action {
            resolve_ref(session_id, input.tab_id, ref_id);
        }
        if is_click {
            // The ref a click lands on becomes what a later, ref-less type/key
            // action is assumed to be typing into.
            redis_store::set_focus(session_id, input.tab_id, ref_id);
        }
        return;
    }

    if is_type_action {
        match redis_store::get_focus(session_id, input.tab_id) {
            Some(ref_id) => resolve_ref(session_id, input.tab_id, &ref_id),
            None => eprintln!(
                "capture: [lookup] type/key with no known focus for this tab - unknown"
            ),
        }
        return;
    }

    if is_ref_lookup_action && input.coordinate.is_some() {
        eprintln!(
            "capture: [lookup] {:?} by coordinate only, no ref - unknown",
            input.action
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // This exact shape is the real tool_response captured from a live
    // claude -p --chrome run of browser_batch(navigate, read_page) - not invented.
    fn real_batch_response() -> serde_json::Value {
        json!([
            {"text": "[navigate] Navigated to http://localhost:8744/login_test.html", "type": "text"},
            {"text": "[read_page] heading \"Sign in\" [ref_1]\nform [ref_2]\n label \"Username\" [ref_3]\n textbox \"Username\" [ref_4] type=\"text\"\n\nViewport: 1470x801", "type": "text"},
            {"text": "\n\nTab Context:\n- Executed on tabId: 427167805", "type": "text"},
        ])
    }

    #[test]
    fn extracts_all_text_blocks_in_order() {
        let blocks = extract_batch_text_blocks(&real_batch_response());
        assert_eq!(blocks.len(), 3);
        assert!(blocks[0].starts_with("[navigate]"));
        assert!(blocks[1].starts_with("[read_page]"));
        assert!(blocks[2].starts_with("\n\nTab Context"));
    }

    #[test]
    fn takes_the_block_matching_the_action_name_and_strips_its_prefix() {
        let mut blocks = extract_batch_text_blocks(&real_batch_response());
        let response = take_batch_action_response(&mut blocks, "read_page").unwrap();
        assert_eq!(
            response.as_str().unwrap(),
            "heading \"Sign in\" [ref_1]\nform [ref_2]\n label \"Username\" [ref_3]\n textbox \"Username\" [ref_4] type=\"text\"\n\nViewport: 1470x801"
        );
        // consumed - a second take for the same name must not find it again
        assert!(take_batch_action_response(&mut blocks, "read_page").is_none());
    }

    #[test]
    fn a_repeated_action_name_gets_its_own_block_not_the_first_one() {
        let mut blocks = vec![
            "[read_page] first call's output".to_string(),
            "[read_page] second call's output".to_string(),
        ];
        let first = take_batch_action_response(&mut blocks, "read_page").unwrap();
        let second = take_batch_action_response(&mut blocks, "read_page").unwrap();
        assert_eq!(first.as_str().unwrap(), "first call's output");
        assert_eq!(second.as_str().unwrap(), "second call's output");
    }

    #[test]
    fn no_matching_block_returns_none() {
        let mut blocks = extract_batch_text_blocks(&real_batch_response());
        assert!(take_batch_action_response(&mut blocks, "find").is_none());
    }
}
