// Stand-in for a real Redis client. Logs exactly what it would do, so the call site and
// the data shape are already right when a real client gets wired in.
// TODO: replace with real Redis calls once Redis exists.

use shared::PageElement;

// The elements hash gets refreshed (TTL reset) on every read_page/find, so 30 min is
// about freshness - it naturally expires 30 min after the LAST read on that tab, since
// stale DOM content shouldn't be trusted longer than that.
const ELEMENTS_TTL_SECONDS: u32 = 30 * 60;

// focus only gets refreshed on clicks, which can be sparser than read_page/find calls,
// so it gets a longer window. This is also the "anything left over expires after 1
// hour" backstop: the explicit delete rules (navigate/tabs_close/etc.) are the primary
// cleanup, but if one of those is ever missed - a tab closed some other way, a crash -
// nothing should live forever regardless.
const FOCUS_TTL_SECONDS: u32 = 60 * 60;

pub fn cache_elements(session_id: &str, tab_id: Option<f64>, elements: &[PageElement]) {
    if elements.is_empty() {
        return;
    }

    let key = match tab_id {
        Some(tab_id) => format!("elements:{session_id}:{tab_id}"),
        None => format!("elements:{session_id}"),
    };

    eprintln!(
        "capture: [redis stub] HSET {key} ({} field(s)), EXPIRE {key} {ELEMENTS_TTL_SECONDS}s",
        elements.len()
    );
    for element in elements {
        eprintln!(
            "capture: [redis stub]   {} => {}",
            element.ref_id, element.description
        );
    }
}

/// Stand-in for looking up a single cached element by ref. Always returns None right
/// now: there's no real store behind this, and capture is a fresh process per hook
/// call, so nothing an earlier invocation "wrote" could ever actually be read back here
/// without one. TODO: replace with a real HGET once Redis exists.
pub fn lookup_element(session_id: &str, tab_id: f64, ref_id: &str) -> Option<String> {
    let key = format!("elements:{session_id}:{tab_id}");
    eprintln!("capture: [redis stub] HGET {key} {ref_id}");
    None
}

/// Stand-in for saving the last-clicked ref for a tab, so a later type/key action
/// (which carries no ref of its own) can resolve what it was typing into.
/// TODO: replace with a real SET once Redis exists.
pub fn set_focus(session_id: &str, tab_id: f64, ref_id: &str) {
    let key = format!("focus:{session_id}:{tab_id}");
    eprintln!("capture: [redis stub] SET {key} {ref_id} EX {FOCUS_TTL_SECONDS}s");
}

/// Stand-in for reading the last-clicked ref back for a tab. Always returns None, same
/// reason as lookup_element. TODO: replace with a real GET once Redis exists.
pub fn get_focus(session_id: &str, tab_id: f64) -> Option<String> {
    let key = format!("focus:{session_id}:{tab_id}");
    eprintln!("capture: [redis stub] GET {key}");
    None
}

/// Stand-in for deleting both maps for one tab: a new page means its old refs are dead
/// (navigate), and a closed tab's map is dead too (tabs_close).
/// TODO: replace with a real DEL once Redis exists.
pub fn delete_tab_maps(session_id: &str, tab_id: f64) {
    eprintln!(
        "capture: [redis stub] DEL elements:{session_id}:{tab_id}, DEL focus:{session_id}:{tab_id}"
    );
}

/// Stand-in for deleting every map for a session - every tab's elements and focus.
/// Used when switching browsers (different browser, different tabs entirely) and on
/// SessionEnd. TODO: replace with a real SCAN + DEL once Redis exists.
pub fn delete_session_maps(session_id: &str) {
    eprintln!(
        "capture: [redis stub] SCAN+DEL elements:{session_id}:* (all tabs), SCAN+DEL focus:{session_id}:* (all tabs)"
    );
}
