use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Effort {
    pub level: String,
}

// 1. PreToolUse: request (Claude Code -> recall)
#[derive(Debug, Deserialize)]
pub struct PreToolUseRequest {
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
    pub tool_name: String,
    pub tool_input: Value,
    pub tool_use_id: String,
}

// 2. PreToolUse: response (recall -> Claude Code)
#[derive(Debug, Serialize, Default)]
pub struct PreToolUseResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#continue: Option<bool>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<PreToolUseHookSpecificOutput>,
}

#[derive(Debug, Serialize, Default)]
pub struct PreToolUseHookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "permissionDecision", skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    #[serde(
        rename = "permissionDecisionReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_decision_reason: Option<String>,
    #[serde(rename = "updatedInput", skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<Value>,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

// 3. PostToolUse: request (Claude Code -> capture)
#[derive(Debug, Deserialize)]
pub struct PostToolUseRequest {
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
    pub tool_name: String,
    pub tool_input: Value,
    pub tool_response: Value,
    pub tool_use_id: String,
}

// 4. PostToolUse: response (capture -> Claude Code)
#[derive(Debug, Serialize, Default)]
pub struct PostToolUseResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#continue: Option<bool>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<PostToolUseHookSpecificOutput>,
}

#[derive(Debug, Serialize, Default)]
pub struct PostToolUseHookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(rename = "updatedToolOutput", skip_serializing_if = "Option::is_none")]
    pub updated_tool_output: Option<Value>,
    #[serde(
        rename = "updatedMCPToolOutput",
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_mcp_tool_output: Option<Value>,
}
