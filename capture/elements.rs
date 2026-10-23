// Parses the plain-text output of claude-in-chrome's read_page and find tools into
// shared::PageElement values - so capture can cache them for later field-type lookups
// (the "by field type" half of the privacy issue, deferred until PostToolUse handling
// existed to carry it).
//
// Both tools return plain text, not JSON - verified against a real page during this
// project, not guessed. read_page lines look like:
//   textbox "Password" [ref_1] type="password"
//   form [ref_7]                          <- no quoted name on every element
// find lines look like:
//   - ref_2: textbox "OTP" (text) - Exact match - textbox with label "OTP"
// find has also been observed doubling the name's quotes on a real page:
//   - ref_853: checkbox ""Non Stop"" (checkbox) - Matches "Non Stop filter checkbox" - ...
// A single "([^"]*)" only matches the first, adjacent quote pair in that case and
// captures an empty name - "+ allows either form without over-matching a normal
// single-quoted name.

use std::sync::LazyLock;

use regex::Regex;
use shared::PageElement;

static READ_PAGE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*(\S+)(?:\s+"+([^"]*)"+)?\s+\[(ref_\d+)\]"#).unwrap());

static FIND_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^-\s*(ref_\d+):\s*(\S+)\s+"+([^"]*)"+"#).unwrap());

pub fn parse_elements(text: &str) -> Vec<PageElement> {
    let mut elements = Vec::new();

    for line in text.lines() {
        if let Some(caps) = FIND_LINE.captures(line) {
            let ref_id = caps[1].to_string();
            let role = &caps[2];
            let name = &caps[3];
            elements.push(PageElement {
                ref_id,
                description: describe(role, Some(name)),
            });
            continue;
        }

        if let Some(caps) = READ_PAGE_LINE.captures(line) {
            let role = &caps[1];
            let name = caps.get(2).map(|m| m.as_str());
            let ref_id = caps[3].to_string();
            elements.push(PageElement {
                ref_id,
                description: describe(role, name),
            });
        }
    }

    elements
}

fn describe(role: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{role} \"{name}\""),
        None => role.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(ref_id: &str, description: &str) -> PageElement {
        PageElement {
            ref_id: ref_id.to_string(),
            description: description.to_string(),
        }
    }

    #[test]
    fn parses_read_page_interactive_output() {
        let text = "textbox \"Password\" [ref_1] type=\"password\"\n\
                     textbox \"OTP\" [ref_2] type=\"text\"\n\
                     \n\
                     Viewport: 1470x801";
        let elements = parse_elements(text);
        assert_eq!(
            elements,
            vec![
                element("ref_1", "textbox \"Password\""),
                element("ref_2", "textbox \"OTP\""),
            ]
        );
    }

    #[test]
    fn parses_read_page_element_with_no_name() {
        let text = "form [ref_7]\n label \"Password\" [ref_8]";
        let elements = parse_elements(text);
        assert_eq!(
            elements,
            vec![element("ref_7", "form"), element("ref_8", "label \"Password\"")]
        );
    }

    #[test]
    fn parses_find_output() {
        let text = "Found 2 matching elements\n\n\
                     - ref_2: textbox \"OTP\" (text) - Exact match - textbox with label \"OTP\"\n\
                     - ref_9: label \"OTP\" (N/A) - Exact match - label for the OTP field";
        let elements = parse_elements(text);
        assert_eq!(
            elements,
            vec![element("ref_2", "textbox \"OTP\""), element("ref_9", "label \"OTP\"")]
        );
    }

    #[test]
    fn ignores_unrelated_text() {
        let elements = parse_elements("just some unrelated page text\nwith no refs at all");
        assert!(elements.is_empty());
    }

    #[test]
    fn parses_find_output_with_doubled_name_quotes() {
        // Captured verbatim from a real find call against makemytrip.com.
        let text = "Found 3 matching elements\n\n\
                     - ref_853: checkbox \"\"Non Stop\"\" (checkbox) - Matches \"Non Stop filter checkbox\" - this is the Non Stop checkbox in the Popular Filters section\n\
                     - ref_859: checkbox \"\"Student Fares\"\" (checkbox) - Located in filters but does not match departure time filters\n\
                     - ref_935: checkbox \"\"Check-in Baggage only\"\" (checkbox) - Located in Baggage section but does not match the requested filters";
        let elements = parse_elements(text);
        assert_eq!(
            elements,
            vec![
                element("ref_853", "checkbox \"Non Stop\""),
                element("ref_859", "checkbox \"Student Fares\""),
                element("ref_935", "checkbox \"Check-in Baggage only\""),
            ]
        );
    }
}
