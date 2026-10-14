// TODO: look up memory once the recall store exists. Returning None means "no hint
// found — leave the caller's response unchanged."
pub fn find_hint<T>(tool_name: &str, tool_input: &T) -> Option<String> {
    let _ = (tool_name, tool_input);

    // Test-only override: there's no real memory store yet, so nothing can exercise the
    // hint-delivery plumbing end-to-end without this. Integration tests set this env var
    // to prove PreToolUseResponse.additionalContext actually reaches Claude; it's unset
    // (and this branch does nothing) outside of that.
    if let Ok(test_hint) = std::env::var("GROOVE_TEST_HINT") {
        return Some(test_hint);
    }

    None
}

/// Gates the (eventual) memory lookup on UserPromptSubmit: only worth asking "is there a
/// hint for this" when the prompt looks like it's headed toward claude-in-chrome at all.
pub fn find_tool_predicate(prompt: &str) -> bool {
    crate::browser_predicate::predict_needs_browser(prompt)
}
