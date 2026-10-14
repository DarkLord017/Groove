use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

#[derive(Debug)]
pub struct PrivacyViolation {
    pub pattern: &'static str,
    pub path: String,
}

impl fmt::Display for PrivacyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.pattern, self.path)
    }
}

const REDACTED: &str = "[REDACTED]";
const REDACTED_SECRET: &str = "[REDACTED_SECRET]";

// Order matters: private tags and private key blocks run first so their contents don't
// also trip the generic secret patterns below, same as upstream's stripPrivateData.
static PATTERNS: LazyLock<Vec<(&'static str, Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            "private tag",
            Regex::new(r"(?s)<private>.*?</private>").unwrap(),
            REDACTED,
        ),
        (
            "private key block",
            Regex::new(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----|$)").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "api key/token/password assignment",
            Regex::new(r#"(?i)(?:api[_-]?key|secret|token|password|credential|auth)\s*[=:]\s*["']?[A-Za-z0-9_\-/.+]{20,}["']?"#).unwrap(),
            REDACTED_SECRET,
        ),
        (
            "bearer token",
            Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-+/=]{20,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "OpenAI project key",
            Regex::new(r"\bsk-proj-[A-Za-z0-9\-_]{20,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "Anthropic key",
            Regex::new(r"\bsk-ant-[A-Za-z0-9\-_]{20,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "generic prefixed key",
            Regex::new(r"\b(?:sk|pk|rk|ak)-[A-Za-z0-9][A-Za-z0-9\-_]{19,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "GitHub token",
            Regex::new(r"gh[pus]_[A-Za-z0-9]{36,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "GitHub PAT",
            Regex::new(r"github_pat_[A-Za-z0-9_]{22,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "Slack token",
            Regex::new(r"xoxb-[A-Za-z0-9\-]+").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "AWS key",
            Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "Google API key",
            Regex::new(r"AIza[A-Za-z0-9\-_]{35}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "JWT",
            Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "npm token",
            Regex::new(r"npm_[A-Za-z0-9]{36}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "GitLab token",
            Regex::new(r"glpat-[A-Za-z0-9\-_]{20,}").unwrap(),
            REDACTED_SECRET,
        ),
        (
            "DigitalOcean token",
            Regex::new(r"dop_v1_[A-Za-z0-9]{64}").unwrap(),
            REDACTED_SECRET,
        ),
    ]
});

// Separate from PATTERNS because its replacement keeps the scheme ("https://") and only
// blanks the credentials, so it needs its own $1-aware replace_all call.
static URL_CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b([a-z][a-z0-9+.\-]*://)[^\s/?#@:"'<>]*:[^\s/?#@"'<>]+@"#).unwrap()
});

/// Walks a JSON value in place and redacts every string leaf. Returns what it found
/// (already redacted) so the caller can report it without re-reading the sensitive value.
pub fn redact(value: &mut Value) -> Vec<PrivacyViolation> {
    let mut violations = Vec::new();
    redact_at("$", value, &mut violations);
    violations
}

/// Same scan/redact as `redact`, for a plain string instead of a JSON tree — for
/// redacting text on its way back out (e.g. a hint) rather than on the way in.
pub fn redact_str(s: &mut String) -> Vec<PrivacyViolation> {
    let mut violations = Vec::new();
    scrub_string("$", s, &mut violations);
    violations
}

fn redact_at(path: &str, value: &mut Value, violations: &mut Vec<PrivacyViolation>) {
    match value {
        Value::String(s) => scrub_string(path, s, violations),
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                redact_at(&format!("{path}[{i}]"), item, violations);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                redact_at(&format!("{path}.{key}"), item, violations);
            }
        }
        _ => {}
    }
}

fn scrub_string(path: &str, s: &mut String, violations: &mut Vec<PrivacyViolation>) {
    if URL_CREDENTIALS.is_match(s) {
        *s = URL_CREDENTIALS
            .replace_all(s, "$1[REDACTED_SECRET]@")
            .into_owned();
        violations.push(PrivacyViolation {
            pattern: "URL credentials",
            path: path.to_string(),
        });
    }

    for (name, regex, replacement) in PATTERNS.iter() {
        if regex.is_match(s) {
            *s = regex.replace_all(s, *replacement).into_owned();
            violations.push(PrivacyViolation {
                pattern: name,
                path: path.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_assignment() {
        let mut s = "api_key=abcdefghijklmnopqrstuvwxyz1234567890".to_string();
        let violations = redact_str(&mut s);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].pattern, "api key/token/password assignment");
        assert_eq!(s, "[REDACTED_SECRET]");
    }

    #[test]
    fn redacts_anthropic_key() {
        let mut s = "key is sk-ant-abcdefghijklmnopqrstuvwxyzABCDEF".to_string();
        let violations = redact_str(&mut s);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].pattern, "Anthropic key");
        assert_eq!(s, "key is [REDACTED_SECRET]");
    }

    #[test]
    fn generic_prefixed_key_does_not_match_mid_token() {
        // "xsk-proj-..." must NOT match - sk- has to start a token, not sit inside one.
        let mut s = "xsk-proj-abcdefghijklmnopqrstuvwxyzABCDEF".to_string();
        let violations = redact_str(&mut s);
        assert!(violations.is_empty());
        assert_eq!(s, "xsk-proj-abcdefghijklmnopqrstuvwxyzABCDEF");
    }

    #[test]
    fn redacts_private_tag_and_everything_inside_it() {
        // private-tag redaction must run before the generic key patterns, so content
        // inside <private> gets wiped wholesale rather than partially redacted.
        let mut s = "note: <private>key is sk-ant-abcdefghijklmnopqrstuvwxyzABCDEF</private>".to_string();
        let violations = redact_str(&mut s);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].pattern, "private tag");
        assert_eq!(s, "note: [REDACTED]");
    }

    #[test]
    fn redacts_url_credentials_but_keeps_the_scheme() {
        let mut s = "go to https://admin:sup3rSecret@internal.example.com/dashboard".to_string();
        let violations = redact_str(&mut s);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].pattern, "URL credentials");
        assert_eq!(
            s,
            "go to https://[REDACTED_SECRET]@internal.example.com/dashboard"
        );
    }

    #[test]
    fn leaves_plain_text_untouched() {
        let mut s = "just a normal ls -la command".to_string();
        let violations = redact_str(&mut s);
        assert!(violations.is_empty());
        assert_eq!(s, "just a normal ls -la command");
    }

    #[test]
    fn redact_walks_nested_json() {
        let mut value = serde_json::json!({
            "tool_input": {"command": "echo hi"},
            "tool_response": {"out": "api_key=abcdefghijklmnopqrstuvwxyz1234567890"},
        });
        let violations = redact(&mut value);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].path, "$.tool_response.out");
        assert_eq!(
            value["tool_response"]["out"],
            serde_json::json!("[REDACTED_SECRET]")
        );
        assert_eq!(value["tool_input"]["command"], serde_json::json!("echo hi"));
    }
}
