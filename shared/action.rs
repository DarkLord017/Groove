// Browser/session-level control: tabs, recording, shortcuts, and which browser is targeted.
// None of these read or mutate page content.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct TabsCreateInput {}

#[derive(Debug, Deserialize, Serialize)]
pub struct TabsCloseInput {
    #[serde(rename = "tabId")]
    pub tab_id: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GifAction {
    StartRecording,
    StopRecording,
    Export,
    Clear,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct GifCreatorOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
    #[serde(rename = "showActionLabels", skip_serializing_if = "Option::is_none")]
    pub show_action_labels: Option<bool>,
    #[serde(
        rename = "showClickIndicators",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_click_indicators: Option<bool>,
    #[serde(rename = "showDragPaths", skip_serializing_if = "Option::is_none")]
    pub show_drag_paths: Option<bool>,
    #[serde(rename = "showProgressBar", skip_serializing_if = "Option::is_none")]
    pub show_progress_bar: Option<bool>,
    #[serde(rename = "showWatermark", skip_serializing_if = "Option::is_none")]
    pub show_watermark: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GifCreatorInput {
    pub action: GifAction,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<GifCreatorOptions>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShortcutsExecuteInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(rename = "shortcutId", skip_serializing_if = "Option::is_none")]
    pub shortcut_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShortcutsListInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SelectBrowserInput {
    #[serde(rename = "deviceId")]
    pub device_id: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SwitchBrowserInput {}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ListConnectedBrowsersInput {}

#[derive(Debug, Deserialize, Serialize)]
pub struct BrowserBatchAction {
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BrowserBatchInput {
    pub actions: Vec<BrowserBatchAction>,
}
