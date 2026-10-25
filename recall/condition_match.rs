use std::collections::HashSet;

// Dropped before comparing - filler words that carry no signal either way and would
// otherwise dilute a real match (or coincidentally "match" on nothing meaningful).
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "be", "of", "to", "on", "in", "for", "with", "and",
    "into", "their", "shown",
];

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// "Is this stage done?" - what fraction of a post_condition's own meaningful words
/// appear anywhere in the current page text. Deliberately NOT the same as word_overlap
/// below: dividing by the condition's own word count (not the union with the page's
/// huge, noisy vocabulary) is what makes this usable - verified on real data
/// (benchmarks/page_condition_match_benchmark.py) after a first attempt (embedding
/// similarity between the raw page text and the condition sentence) got the ranking
/// backwards entirely.
pub fn page_satisfies_condition(page_text: &str, condition: &str) -> f32 {
    let condition_words = words(condition);
    if condition_words.is_empty() {
        return 0.0;
    }
    let page_words = words(page_text);
    let found = condition_words.intersection(&page_words).count();
    found as f32 / condition_words.len() as f32
}

/// "Does this candidate pick up where the last stage left off?" - plain Jaccard word
/// overlap between two condition sentences (both written by the same stage_llm, same
/// register - unlike page_satisfies_condition's raw-page-text comparison). Mirrors
/// site_tree.py's own _jaccard exactly: no stopword filtering there either, since both
/// sides are short, already-clean sentences, not a noisy page dump.
pub fn word_overlap(a: &str, b: &str) -> f32 {
    let tokens_a: HashSet<String> = a.to_lowercase().split_whitespace().map(str::to_string).collect();
    let tokens_b: HashSet<String> = b.to_lowercase().split_whitespace().map(str::to_string).collect();
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }
    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();
    intersection as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real captures from claude-in-chrome's get_page_text against a local test page
    // (empty search form -> click Search -> results appear) - see
    // benchmarks/page_condition_match_benchmark.py for where these came from.
    const BEFORE_SEARCH: &str = "Flight Search\nOrigin Destination Search";
    const AFTER_SEARCH: &str = "Flight Search\nOrigin Destination Search\nSearch Results\nFlight AI101 - Mumbai to Delhi - $120\nFlight AI202 - Mumbai to Delhi - $145\nFlight AI303 - Mumbai to Delhi - $98";

    #[test]
    fn page_satisfies_condition_distinguishes_before_and_after() {
        let condition = "Search results are shown";
        let before = page_satisfies_condition(BEFORE_SEARCH, condition);
        let after = page_satisfies_condition(AFTER_SEARCH, condition);
        assert!(before < after, "before={before} should be less than after={after}");
        assert_eq!(after, 1.0);
    }

    #[test]
    fn page_satisfies_condition_rejects_unrelated_condition() {
        let score = page_satisfies_condition(AFTER_SEARCH, "User is logged into their account");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn word_overlap_is_perfect_for_identical_sentences() {
        assert_eq!(word_overlap("Origin and destination fields are filled", "Origin and destination fields are filled"), 1.0);
    }

    #[test]
    fn word_overlap_is_low_for_unrelated_sentences() {
        let score = word_overlap("Origin and destination fields are filled", "Search results are shown");
        assert!(score < 0.2, "score={score} should be low");
    }
}
