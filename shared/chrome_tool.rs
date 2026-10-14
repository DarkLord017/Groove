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

#[derive(Debug)]
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
