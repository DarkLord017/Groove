// Collapses the 9 hook events into one shape so capture can log/persist them all the
// same way. Everything here is derived from an already-redacted payload (see
// shared::privacy) — the normalizer does no redaction of its own.

use serde_json::{json, Value};
use shared::HookEvent;

#[derive(Debug)]
pub struct NormalizedEvent {
    pub session_id: String,
    pub hook_event_name: String,
    pub tool_name: Option<String>,
    pub payload: Value,
}

pub fn normalize(event: &HookEvent) -> NormalizedEvent {
    match event {
        HookEvent::SessionStart(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "source": req.source }),
        },
        HookEvent::UserPromptSubmit(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "prompt": req.prompt }),
        },
        HookEvent::PreToolUse(req) => NormalizedEvent {
            session_id: req.session_id.clone(),
            hook_event_name: req.hook_event_name.clone(),
            tool_name: Some(req.tool_name.clone()),
            payload: req.tool_input.clone(),
        },
        HookEvent::PostToolUse(req) => NormalizedEvent {
            session_id: req.session_id.clone(),
            hook_event_name: req.hook_event_name.clone(),
            tool_name: Some(req.tool_name.clone()),
            payload: json!({
                "tool_input": req.tool_input,
                "tool_response": req.tool_response,
            }),
        },
        HookEvent::Notification(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({
                "notification_type": req.notification_type,
                "message": req.message,
            }),
        },
        HookEvent::Stop(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "last_assistant_message": req.last_assistant_message }),
        },
        HookEvent::SubagentStop(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "last_assistant_message": req.last_assistant_message }),
        },
        HookEvent::PreCompact(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "trigger": req.trigger }),
        },
        HookEvent::SessionEnd(req) => NormalizedEvent {
            session_id: req.common.session_id.clone(),
            hook_event_name: req.common.hook_event_name.clone(),
            tool_name: None,
            payload: json!({ "reason": req.reason }),
        },
    }
}
