use std::fmt;

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

const SENSITIVE_PREFIXES: &[&str] = &[
    "sk-", "ghp_", "gho_", "github_pat_", "AKIA", "-----BEGIN", "xox",
];

/// Walks a JSON value in place and redacts any string containing a known secret-like
/// prefix, replacing it with "[REDACTED]". Returns what it found (already redacted)
/// so the caller can report it without re-reading the sensitive value.
pub fn redact(value: &mut Value) -> Vec<PrivacyViolation> {
    let mut violations = Vec::new();
    redact_at("$", value, &mut violations);
    violations
}

fn redact_at(path: &str, value: &mut Value, violations: &mut Vec<PrivacyViolation>) {
    match value {
        Value::String(s) => {
            if let Some(&pattern) = SENSITIVE_PREFIXES.iter().find(|p| s.contains(**p)) {
                violations.push(PrivacyViolation {
                    pattern,
                    path: path.to_string(),
                });
                *s = "[REDACTED]".to_string();
            }
        }
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
