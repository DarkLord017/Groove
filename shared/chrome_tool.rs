use serde::Serialize;
use serde_json::Value;

use crate::action::{
    BrowserBatchInput, GifCreatorInput, ListConnectedBrowsersInput, SelectBrowserInput,
    ShortcutsExecuteInput, ShortcutsListInput, SwitchBrowserInput, TabsCloseInput,
    TabsCreateInput,
};
use crate::page_changes::{
    ComputerInput, FileUploadInput, FormInputInput, JavascriptToolInput, NavigateInput,
    ResizeWindowInput, UploadImageInput,
};
use crate::page_reads::{
    FindInput, GetPageTextInput, ReadConsoleMessagesInput, ReadNetworkRequestsInput,
    ReadPageInput, TabsContextInput,
};

#[derive(Debug, Serialize)]
pub enum ChromeToolInput {
    TabsContext(TabsContextInput),
    Navigate(NavigateInput),
    Computer(ComputerInput),
    ReadPage(ReadPageInput),
    TabsCreate(TabsCreateInput),
    TabsClose(TabsCloseInput),
    Find(FindInput),
    GetPageText(GetPageTextInput),
    FormInput(FormInputInput),
    JavascriptTool(JavascriptToolInput),
    GifCreator(GifCreatorInput),
    FileUpload(FileUploadInput),
    ReadConsoleMessages(ReadConsoleMessagesInput),
    ReadNetworkRequests(ReadNetworkRequestsInput),
    ResizeWindow(ResizeWindowInput),
    BrowserBatch(BrowserBatchInput),
    ShortcutsExecute(ShortcutsExecuteInput),
    ShortcutsList(ShortcutsListInput),
    SelectBrowser(SelectBrowserInput),
    SwitchBrowser(SwitchBrowserInput),
    ListConnectedBrowsers(ListConnectedBrowsersInput),
    UploadImage(UploadImageInput),
    /// Not a claude-in-chrome tool, or a tool this crate doesn't model yet.
    Unknown(Value),
}

impl ChromeToolInput {
    pub fn from_tool_call(tool_name: &str, tool_input: Value) -> serde_json::Result<Self> {
        Ok(match tool_name {
            "mcp__claude-in-chrome__tabs_context_mcp" => {
                Self::TabsContext(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__navigate" => Self::Navigate(serde_json::from_value(tool_input)?),
            "mcp__claude-in-chrome__computer" => Self::Computer(serde_json::from_value(tool_input)?),
            "mcp__claude-in-chrome__read_page" => Self::ReadPage(serde_json::from_value(tool_input)?),
            "mcp__claude-in-chrome__tabs_create_mcp" => {
                Self::TabsCreate(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__tabs_close_mcp" => {
                Self::TabsClose(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__find" => Self::Find(serde_json::from_value(tool_input)?),
            "mcp__claude-in-chrome__get_page_text" => {
                Self::GetPageText(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__form_input" => {
                Self::FormInput(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__javascript_tool" => {
                Self::JavascriptTool(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__gif_creator" => {
                Self::GifCreator(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__file_upload" => {
                Self::FileUpload(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__read_console_messages" => {
                Self::ReadConsoleMessages(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__read_network_requests" => {
                Self::ReadNetworkRequests(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__resize_window" => {
                Self::ResizeWindow(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__browser_batch" => {
                Self::BrowserBatch(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__shortcuts_execute" => {
                Self::ShortcutsExecute(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__shortcuts_list" => {
                Self::ShortcutsList(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__select_browser" => {
                Self::SelectBrowser(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__switch_browser" => {
                Self::SwitchBrowser(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__list_connected_browsers" => {
                Self::ListConnectedBrowsers(serde_json::from_value(tool_input)?)
            }
            "mcp__claude-in-chrome__upload_image" => {
                Self::UploadImage(serde_json::from_value(tool_input)?)
            }
            _ => Self::Unknown(tool_input),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_changes::ComputerAction;
    use serde_json::json;

    // These payloads are the real shapes Claude Code actually sent during a live
    // claude -p --chrome run (captured via --include-hook-events), not guessed - see
    // the evals/pretooluse_recall_test.sh verification. Regression-pins them so a
    // schema drift fails here, fast and free, instead of only ever showing up in a
    // live run.

    #[test]
    fn parses_navigate() {
        let input = json!({"url": "http://localhost:8743/login_test.html"});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__navigate", input).unwrap();
        assert!(matches!(result, ChromeToolInput::Navigate(NavigateInput { .. })));
    }

    #[test]
    fn parses_find() {
        let input = json!({"query": "username and password input fields", "tabId": 427167511});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__find", input).unwrap();
        assert!(matches!(result, ChromeToolInput::Find(FindInput { .. })));
    }

    #[test]
    fn parses_form_input() {
        let input = json!({"ref": "ref_5", "value": "alice", "tabId": 427167511});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__form_input", input).unwrap();
        match result {
            ChromeToolInput::FormInput(form) => {
                assert_eq!(form.r#ref, "ref_5");
                assert_eq!(form.value, json!("alice"));
            }
            other => panic!("expected FormInput, got {other:?}"),
        }
    }

    #[test]
    fn parses_computer_left_click() {
        let input = json!({"action": "left_click", "ref": "ref_8", "tabId": 427167511});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__computer", input).unwrap();
        match result {
            ChromeToolInput::Computer(computer) => {
                assert!(matches!(computer.action, ComputerAction::LeftClick));
            }
            other => panic!("expected Computer, got {other:?}"),
        }
    }

    #[test]
    fn parses_computer_screenshot_with_no_ref() {
        let input = json!({"action": "screenshot", "tabId": 427167511});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__computer", input).unwrap();
        assert!(matches!(result, ChromeToolInput::Computer(_)));
    }

    #[test]
    fn parses_read_page() {
        let input = json!({"tabId": 427167511, "filter": "interactive"});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__read_page", input).unwrap();
        assert!(matches!(result, ChromeToolInput::ReadPage(_)));
    }

    #[test]
    fn non_chrome_tool_is_unknown_not_an_error() {
        let input = json!({"command": "ls"});
        let result = ChromeToolInput::from_tool_call("Bash", input).unwrap();
        assert!(matches!(result, ChromeToolInput::Unknown(_)));
    }

    #[test]
    fn known_tool_with_wrong_shape_is_an_error_not_unknown() {
        // url as a number instead of a string - a real schema mismatch, which must come
        // back as Err, not silently fall into Unknown like a non-chrome tool would.
        let input = json!({"url": 12345, "tabId": 1});
        let result = ChromeToolInput::from_tool_call("mcp__claude-in-chrome__navigate", input);
        assert!(result.is_err());
    }
}
