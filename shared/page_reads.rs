// Tools that only observe page/tab/console/network state; they never mutate the page.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadPageFilter {
    Interactive,
    All,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct TabsContextInput {
    #[serde(rename = "createIfEmpty", skip_serializing_if = "Option::is_none")]
    pub create_if_empty: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadPageInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<ReadPageFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindInput {
    pub query: String,
    #[serde(rename = "tabId")]
    pub tab_id: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetPageTextInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadConsoleMessagesInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(rename = "onlyErrors", skip_serializing_if = "Option::is_none")]
    pub only_errors: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadNetworkRequestsInput {
    #[serde(rename = "tabId")]
    pub tab_id: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(rename = "urlPattern", skip_serializing_if = "Option::is_none")]
    pub url_pattern: Option<String>,
}

/// A single element parsed out of read_page's or find's plain-text tool_response, e.g.
/// {ref_id: "ref_17", description: "textbox \"First name\""}. Not deserialized from
/// JSON - both tools return plain text, not JSON - this is the typed shape that text
/// gets parsed into, named here so it's part of the shared vocabulary rather than an
/// anonymous tuple scattered wherever it's produced or consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageElement {
    pub ref_id: String,
    pub description: String,
}
