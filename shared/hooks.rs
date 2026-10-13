use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::r#struct::{Effort, PostToolUseRequest, PreToolUseRequest};

#[derive(Debug, Deserialize)]
pub struct CommonHookFields {
    pub session_id: String,
    pub prompt_id: Option<String>,
    pub transcript_path: String,
    pub cwd: String,
    pub scratchpad_dir: Option<String>,
    pub permission_mode: Option<String>,
    pub effort: Option<Effort>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub hook_event_name: String,
}

#[derive(Debug, Serialize, Default)]
pub struct CommonHookOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#continue: Option<bool>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct AdditionalContextOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

// ===== SessionStart =====

#[derive(Debug, Deserialize)]
pub struct SessionStartRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub source: String, // "startup" | "resume" | "clear" | "compact" | "fork"
    pub model: Option<String>,
    pub session_title: Option<String>,
    pub seconds_since_last_response: Option<f64>,
    pub context_tokens: Option<f64>,
    pub prompt_cache_likely_expired: Option<bool>,
    pub estimated_cache_write_usd: Option<f64>,
}

#[derive(Debug, Serialize, Default)]
pub struct SessionStartHookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(rename = "initialUserMessage", skip_serializing_if = "Option::is_none")]
    pub initial_user_message: Option<String>,
    #[serde(rename = "sessionTitle", skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    #[serde(rename = "watchPaths", skip_serializing_if = "Option::is_none")]
    pub watch_paths: Option<Vec<String>>,
    #[serde(rename = "reloadSkills", skip_serializing_if = "Option::is_none")]
    pub reload_skills: Option<bool>,
}

#[derive(Debug, Serialize, Default)]
pub struct SessionStartResponse {
    #[serde(flatten)]
    pub common: CommonHookOutput,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<SessionStartHookSpecificOutput>,
}

// ===== UserPromptSubmit =====

#[derive(Debug, Deserialize)]
pub struct UserPromptSubmitRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub prompt: String,
}

#[derive(Debug, Serialize, Default)]
pub struct UserPromptSubmitResponse {
    #[serde(flatten)]
    pub common: CommonHookOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>, // "block"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<AdditionalContextOutput>,
}

// ===== Notification =====
// No decision control: exit code and JSON response are ignored, so there is no
// matching Response type.

#[derive(Debug, Deserialize)]
pub struct NotificationRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub notification_type: String,
    pub message: String,
}

// ===== Stop =====

#[derive(Debug, Deserialize)]
pub struct StopRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub last_assistant_message: String,
}

#[derive(Debug, Serialize, Default)]
pub struct StopResponse {
    #[serde(flatten)]
    pub common: CommonHookOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>, // "block"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<AdditionalContextOutput>,
}

// ===== SubagentStop =====
// Same input/output shape as Stop, scoped to a subagent's own transcript.

#[derive(Debug, Deserialize)]
pub struct SubagentStopRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub last_assistant_message: String,
}

pub type SubagentStopResponse = StopResponse;

// ===== PreCompact =====

#[derive(Debug, Deserialize)]
pub struct PreCompactRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub trigger: String, // "manual" | "auto"
}

#[derive(Debug, Serialize, Default)]
pub struct PreCompactResponse {
    #[serde(flatten)]
    pub common: CommonHookOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>, // "block"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ===== SessionEnd =====
// No decision control, so there is no matching Response type.

#[derive(Debug, Deserialize)]
pub struct SessionEndRequest {
    #[serde(flatten)]
    pub common: CommonHookFields,
    pub reason: String, // "clear" | "resume" | "logout" | "prompt_input_exit" | "other"
}

// ===== Dispatch =====
// Peeks hook_event_name on the raw payload and deserializes into the matching
// request type above (or the existing PreToolUseRequest/PostToolUseRequest).

#[derive(Debug)]
pub enum HookEvent {
    SessionStart(SessionStartRequest),
    UserPromptSubmit(UserPromptSubmitRequest),
    PreToolUse(PreToolUseRequest),
    PostToolUse(PostToolUseRequest),
    Notification(NotificationRequest),
    Stop(StopRequest),
    SubagentStop(SubagentStopRequest),
    PreCompact(PreCompactRequest),
    SessionEnd(SessionEndRequest),
}

impl HookEvent {
    pub fn from_value(raw: Value) -> serde_json::Result<Self> {
        let hook_event_name = raw
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        Ok(match hook_event_name.as_str() {
            "SessionStart" => Self::SessionStart(serde_json::from_value(raw)?),
            "UserPromptSubmit" => Self::UserPromptSubmit(serde_json::from_value(raw)?),
            "PreToolUse" => Self::PreToolUse(serde_json::from_value(raw)?),
            "PostToolUse" => Self::PostToolUse(serde_json::from_value(raw)?),
            "Notification" => Self::Notification(serde_json::from_value(raw)?),
            "Stop" => Self::Stop(serde_json::from_value(raw)?),
            "SubagentStop" => Self::SubagentStop(serde_json::from_value(raw)?),
            "PreCompact" => Self::PreCompact(serde_json::from_value(raw)?),
            "SessionEnd" => Self::SessionEnd(serde_json::from_value(raw)?),
            other => {
                return Err(serde::de::Error::custom(format!(
                    "unknown hook_event_name: {other}"
                )))
            }
        })
    }
}
