use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::chrome_tool::ChromeToolInput;
use crate::page_changes::ComputerAction;

pub const QUEUE_COUNT: u32 = 32;

pub fn queue_for_session(session_id: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    (hasher.finish() % QUEUE_COUNT as u64) as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RabbitMessageKind {
    PageSnapshot,
    Steps,
    Prompt,
}

impl RabbitMessageKind {
    pub fn routing_key(self) -> &'static str {
        match self {
            RabbitMessageKind::PageSnapshot => "page.snapshot",
            RabbitMessageKind::Steps => "steps",
            RabbitMessageKind::Prompt => "prompt",
        }
    }
}


pub fn message_kind(tool_input: &ChromeToolInput) -> Option<RabbitMessageKind> {
    use ChromeToolInput::*;
    match tool_input {
        ReadPage(_) | Find(_) => Some(RabbitMessageKind::PageSnapshot),
        Navigate(_) | TabsCreate(_) | TabsClose(_) | FormInput(_) | FileUpload(_)
        | UploadImage(_) | ShortcutsExecute(_) => Some(RabbitMessageKind::Steps),
        Computer(input) => match input.action {
            ComputerAction::LeftClick
            | ComputerAction::RightClick
            | ComputerAction::DoubleClick
            | ComputerAction::TripleClick
            | ComputerAction::Hover
            | ComputerAction::Type
            | ComputerAction::Key
            | ComputerAction::Scroll
            | ComputerAction::LeftClickDrag => Some(RabbitMessageKind::Steps),
            ComputerAction::Screenshot
            | ComputerAction::Wait
            | ComputerAction::Zoom
            | ComputerAction::ScrollTo => None,
        },
        _ => None,
    }
}

/// FileUpload/UploadImage publish a marker only - never the file path, image id, or
/// filename. Returns the safe marker payload for these two; None for everything else
/// (meaning: publish the normal tool_input/tool_response payload instead).
pub fn marker_payload(tool_input: &ChromeToolInput) -> Option<serde_json::Value> {
    use ChromeToolInput::*;
    match tool_input {
        FileUpload(input) => Some(serde_json::json!({
            "marker": "file_upload",
            "ref": input.r#ref,
            "tab_id": input.tab_id,
        })),
        UploadImage(input) => Some(serde_json::json!({
            "marker": "upload_image",
            "ref": input.r#ref,
            "tab_id": input.tab_id,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_changes::{ComputerInput, FileUploadInput};
    use serde_json::json;

    #[test]
    fn read_page_and_find_are_page_snapshot() {
        let read_page = ChromeToolInput::from_tool_call(
            "mcp__claude-in-chrome__read_page",
            json!({"tabId": 1}),
        )
        .unwrap();
        assert_eq!(message_kind(&read_page), Some(RabbitMessageKind::PageSnapshot));

        let find = ChromeToolInput::from_tool_call(
            "mcp__claude-in-chrome__find",
            json!({"query": "x", "tabId": 1}),
        )
        .unwrap();
        assert_eq!(message_kind(&find), Some(RabbitMessageKind::PageSnapshot));
    }

    #[test]
    fn navigate_tabs_create_tabs_close_form_input_are_steps() {
        let navigate =
            ChromeToolInput::from_tool_call("mcp__claude-in-chrome__navigate", json!({"url": "https://x.com"}))
                .unwrap();
        assert_eq!(message_kind(&navigate), Some(RabbitMessageKind::Steps));
    }

    #[test]
    fn computer_click_type_scroll_are_steps_but_screenshot_wait_zoom_scroll_to_are_not() {
        for action in ["left_click", "right_click", "double_click", "triple_click", "hover", "type", "key", "scroll", "left_click_drag"] {
            let input = ChromeToolInput::from_tool_call(
                "mcp__claude-in-chrome__computer",
                json!({"action": action, "tabId": 1}),
            )
            .unwrap();
            assert_eq!(message_kind(&input), Some(RabbitMessageKind::Steps), "action {action} should be steps");
        }
        for action in ["screenshot", "wait", "zoom", "scroll_to"] {
            let input = ChromeToolInput::from_tool_call(
                "mcp__claude-in-chrome__computer",
                json!({"action": action, "tabId": 1}),
            )
            .unwrap();
            assert_eq!(message_kind(&input), None, "action {action} should not be forwarded");
        }
    }

    #[test]
    fn select_browser_switch_browser_shortcuts_list_are_not_forwarded() {
        let select = ChromeToolInput::from_tool_call(
            "mcp__claude-in-chrome__select_browser",
            json!({"deviceId": "d1"}),
        )
        .unwrap();
        assert_eq!(message_kind(&select), None);

        let switch =
            ChromeToolInput::from_tool_call("mcp__claude-in-chrome__switch_browser", json!({})).unwrap();
        assert_eq!(message_kind(&switch), None);

        let shortcuts_list =
            ChromeToolInput::from_tool_call("mcp__claude-in-chrome__shortcuts_list", json!({"tabId": 1}))
                .unwrap();
        assert_eq!(message_kind(&shortcuts_list), None);
    }

    #[test]
    fn file_upload_gets_a_marker_payload_with_no_paths() {
        let input = ChromeToolInput::FileUpload(FileUploadInput {
            paths: vec!["/secret/path/resume.pdf".to_string()],
            r#ref: "ref_9".to_string(),
            tab_id: 1.0,
        });
        let payload = marker_payload(&input).unwrap();
        assert_eq!(payload["marker"], "file_upload");
        assert_eq!(payload["ref"], "ref_9");
        assert!(payload.to_string().find("resume.pdf").is_none());
    }

    #[test]
    fn non_upload_tools_have_no_marker_payload() {
        let input = ChromeToolInput::Computer(ComputerInput {
            action: ComputerAction::LeftClick,
            tab_id: 1.0,
            coordinate: None,
            start_coordinate: None,
            region: None,
            text: None,
            duration: None,
            modifiers: None,
            r#ref: Some("ref_1".to_string()),
            repeat: None,
            save_to_disk: None,
            scale: None,
            scroll_amount: None,
            scroll_direction: None,
        });
        assert!(marker_payload(&input).is_none());
    }

    #[test]
    fn queue_for_session_is_stable_and_within_pool() {
        let q1 = queue_for_session("session-abc");
        let q2 = queue_for_session("session-abc");
        assert_eq!(q1, q2, "same session must always hash to the same queue");
        assert!(q1 < QUEUE_COUNT);
    }
}
