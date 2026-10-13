use crate::chrome_tool::ChromeToolInput;

// TODO: look up memory once the recall store exists. Returning None means "no hint
// found — leave the caller's response unchanged."
pub fn find_hint(tool_name: &str, tool_input: &ChromeToolInput) -> Option<String> {
    let _ = (tool_name, tool_input);
    None
}
