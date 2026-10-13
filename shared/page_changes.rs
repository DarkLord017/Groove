// Tools that mutate page content/DOM state or drive input into the page.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Coordinate = (f64, f64);

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAction {
    LeftClick,
    RightClick,
    Type,
    Screenshot,
    Wait,
    Scroll,
    Key,
    LeftClickDrag,
    DoubleClick,
    TripleClick,
    Zoom,
    ScrollTo,
    Hover,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NavigateInput {
    pub url: String,
    #[serde(rename = "tabId", skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ComputerInput {
    pub action: ComputerAction,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<Coordinate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_coordinate: Option<Coordinate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_to_disk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_direction: Option<ScrollDirection>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FormInputInput {
    #[serde(rename = "ref")]
    pub r#ref: String,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    pub value: Value, // string | bool | number
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FileUploadInput {
    pub paths: Vec<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UploadImageInput {
    #[serde(rename = "imageId")]
    pub image_id: String,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<Coordinate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JavascriptToolInput {
    pub action: String, // always "javascript_exec"
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    pub text: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResizeWindowInput {
    pub width: f64,
    pub height: f64,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
}
