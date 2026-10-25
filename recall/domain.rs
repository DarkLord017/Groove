use url::Url;

/// Mirrors learn/src/learn/hmt/site_tree.py's set_domain: hostname, "www." stripped,
/// lowercased. recall and learn must agree on this exactly - learn writes stage/intent
/// points keyed by this normalized domain, so a mismatch here means a real, learned site
/// would never look "known" from recall's side.
pub fn normalize(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    Some(host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_www_and_lowercases() {
        assert_eq!(normalize("https://WWW.Example.com/path?q=1"), Some("example.com".to_string()));
    }

    #[test]
    fn leaves_a_bare_host_alone() {
        assert_eq!(normalize("http://example.com"), Some("example.com".to_string()));
    }

    #[test]
    fn rejects_unparseable_input() {
        assert_eq!(normalize("not a url"), None);
    }
}
