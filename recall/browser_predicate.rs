
use std::sync::LazyLock;

use regex::Regex;

static URL_OR_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://|www\.[a-z0-9-]+\.[a-z]{2,}|\b[a-z0-9-]+\.(com|io|dev|net|org|co|app|ai)\b")
        .unwrap()
});

const BROWSER_KEYWORDS: &[&str] = &[
    "website",
    "webpage",
    "web page",
    "browser",
    "chrome",
    "navigate",
    "log in",
    "login",
    "sign in",
    "signin",
    "click",
    "open this page",
    "open the page",
    "check out this site",
    "check out this page",
    "fill in the form",
    "form field",
];

// Task-intent phrases: the prompt never mentions a browser, page, or URL at all, but the
// task itself (booking, shopping, price-checking) is realistically only doable by
// navigating a real site.
const TASK_INTENT_KEYWORDS: &[&str] = &[
    "book a",
    "book me a",
    "book us a",
    "buy a",
    "buy me a",
    "purchase a",
    "order a",
    "order me a",
    "reserve a",
    "shop for",
    "add to cart",
    "price of",
    "prices of",
    "how much does",
    "how much is",
    "flight to",
    "flight from",
    "flights to",
    "flights from",
    "hotel in",
    "hotel near",
    "restaurant reservation",
    "find me a",
];

/// Predicts whether a prompt is likely to need claude-in-chrome. Errs toward true: a
/// missed hint (false negative) is a worse outcome than a wasted, cheap lookup (false
/// positive), since there's no real memory store behind it yet for the cost to matter.
pub fn predict_needs_browser(prompt: &str) -> bool {
    if URL_OR_DOMAIN.is_match(prompt) {
        return true;
    }

    let lower = prompt.to_lowercase();
    BROWSER_KEYWORDS.iter().any(|keyword| lower.contains(keyword))
        || TASK_INTENT_KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_url() {
        assert!(predict_needs_browser(
            "check out https://example.com and log in for me"
        ));
    }

    #[test]
    fn detects_a_bare_domain() {
        assert!(predict_needs_browser("go to example.com and sign up"));
    }

    #[test]
    fn detects_browser_keywords_without_a_url() {
        assert!(predict_needs_browser(
            "please log in to the website for me"
        ));
    }

    #[test]
    fn detects_task_intent_with_no_browser_language_at_all() {
        assert!(predict_needs_browser("I want to book a flight to Tokyo"));
    }

    #[test]
    fn detects_price_check_task_intent() {
        assert!(predict_needs_browser("how much does the new iPhone cost"));
    }

    #[test]
    fn rejects_unrelated_prompt() {
        assert!(!predict_needs_browser(
            "fix the off-by-one bug in this rust loop"
        ));
    }

    #[test]
    fn rejects_prompt_with_a_secret_but_no_browser_signal() {
        assert!(!predict_needs_browser(
            "explain how linked lists work in one sentence, separately here is a test string sk-testsecretvalueabcdefghijklmnop for fixture data"
        ));
    }
}
