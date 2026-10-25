use shared::{ChromeToolInput, FormInputInput, NavigateInput};

use crate::condition_match;

/// Test-only override: there's no cheap way to exercise the hint-delivery plumbing
/// end-to-end (real Qdrant data, a real prior prompt, a real learned site) from the
/// integration tests under evals/, so this lets them prove
/// PreToolUseResponse.additionalContext / UserPromptSubmitResponse.additionalContext
/// actually reach Claude without depending on any of that. Checked first and
/// unconditionally - unset outside of that, so this branch does nothing normally.
fn test_override() -> Option<String> {
    std::env::var("GROOVE_TEST_HINT").ok()
}

/// PreToolUse entry point: `navigate` drives the "navigate -> plan" leg, `form_input`
/// drives the diagram's "Action picker" leg (suggest a saved fact's value for a
/// textbox/checkbox/option about to be filled). Every other claude-in-chrome tool is a
/// no-op here.
pub fn find_hint(session_id: &str, tool_input: &ChromeToolInput) -> Option<String> {
    if let Some(hint) = test_override() {
        return Some(hint);
    }

    match tool_input {
        ChromeToolInput::Navigate(navigate) => find_navigate_hint(session_id, navigate),
        ChromeToolInput::FormInput(form_input) => find_fact_hint(session_id, form_input),
        _ => None,
    }
}

/// Resolves the field's ref to its human-readable label (same cache capture itself
/// writes from earlier read_page/find calls - e.g. `textbox "Home city"`), then matches
/// it against saved facts. Silent whenever the ref isn't cached (nothing read the page
/// recently enough) or nothing matches confidently - same "say nothing rather than
/// guess" rule as every other hint in this system.
fn find_fact_hint(session_id: &str, form_input: &FormInputInput) -> Option<String> {
    let label = redis_store::lookup_element(session_id, form_input.tab_id, &form_input.r#ref)?;
    let fact = qdrant_store::match_fact(&label)?;
    Some(format!(
        "Saved value on file for {label}: \"{}\" (from {}, match score {:.2}). If other \
        fields on this page also have saved values, set them together in one \
        browser_batch call instead of one form_input call per field.\n",
        fact.value, fact.key, fact.score,
    ))
}

/// UserPromptSubmit entry point. No real logic yet beyond the test override - see
/// find_tool_predicate's docstring for why this is only even called when the prompt looks
/// browser-bound in the first place.
pub fn find_prompt_hint(_prompt: &str) -> Option<String> {
    test_override()
}

/// Gates the (eventual) memory lookup on UserPromptSubmit: only worth asking "is there a
/// hint for this" when the prompt looks like it's headed toward claude-in-chrome at all.
pub fn find_tool_predicate(prompt: &str) -> bool {
    crate::browser_predicate::predict_needs_browser(prompt)
}

// Diagram's "navigate" leg. AWM first: a confident workflow match is already the full
// answer (a ready-made step list), so HMT's plan listing is only a fallback for when AWM
// has nothing. Either way, the HMT intent gets resolved and cached - even on the AWM path
// - as insurance for when an AWM workflow runs out of steps partway through a real run and
// HMT needs to take over; nothing else happens with it on that path right now.
fn find_navigate_hint(session_id: &str, navigate: &NavigateInput) -> Option<String> {
    let domain = crate::domain::normalize(&navigate.url)?;
    let prompt = redis_store::get_recent_prompts(session_id, 1).into_iter().next();

    let hmt_intent = resolve_and_cache_intent(session_id, navigate.tab_id, &domain, prompt.as_deref());

    if let Some(prompt) = &prompt {
        if qdrant_store::awm_known_site(&domain) {
            if let Some(workflow) = qdrant_store::match_workflow(&domain, prompt) {
                return Some(render_workflow(&domain, &workflow));
            }
        }
    }

    // HMT fallback - reuses the intent already resolved above rather than re-querying.
    let intent = hmt_intent?;
    let stages = qdrant_store::known_stages(&domain, &intent);
    if stages.is_empty() {
        return None;
    }
    Some(render_plan(&domain, &intent, &stages))
}

/// Resolves the HMT intent for (domain, prompt) and caches it (with the domain) for this
/// (session, tab), unconditionally - see find_navigate_hint's docs. Only cacheable with a
/// tab_id; without one this just resolves without caching (same precedent as capture's
/// delete_tab_maps: skip the cache rather than guess which tab it belongs to).
fn resolve_and_cache_intent(
    session_id: &str,
    tab_id: Option<f64>,
    domain: &str,
    prompt: Option<&str>,
) -> Option<String> {
    if !qdrant_store::hmt_known_site(domain) {
        return None;
    }
    let intent = qdrant_store::match_intent(domain, prompt?);

    match (tab_id, &intent) {
        (Some(tab_id), Some(intent)) => {
            redis_store::set_intent(session_id, tab_id, intent);
            redis_store::set_tab_domain(session_id, tab_id, domain);
        }
        (Some(tab_id), None) => redis_store::clear_intent(session_id, tab_id),
        (None, _) => eprintln!("recall: navigate with no explicit tabId - cannot cache resolved intent"),
    }

    intent
}

// Same thresholds site_tree.py's select_stage already uses - not re-derived here, just
// ported alongside the formula they belong to.
const STAGE_MIN_CONFIDENCE: f32 = 0.15;
const STAGE_MIN_MARGIN: f32 = 0.1;
const STAGE_COND_WEIGHT: f32 = 0.3;

// A post_condition counts as "now true" only when every one of its own meaningful words
// shows up on the page - the strictest reading of page_satisfies_condition's 0.0-1.0
// scale. Chosen conservatively: benchmarks/page_condition_match_benchmark.py only
// verified one real true-positive (1.0) and one real false-positive (0.5) so far, not
// anything in between - wrongly marking a stage done (and steering the agent away from
// work it still needs to do) is worse than staying quiet on a real but partial match, so
// this leans toward missing a few until there's more real data to tune on.
const PAGE_DONE_THRESHOLD: f32 = 1.0;

/// PostToolUse entry point for the `ReadPage -> stage` leg: any claude-in-chrome call
/// that hands back real page text (read_page, find, get_page_text) can feed this. Needs
/// the intent+domain find_navigate_hint already cached for this tab - without that
/// there's nothing to check stages against, so this stays quiet rather than guess.
pub fn find_stage_hint(session_id: &str, tab_id: f64, page_text: &str) -> Option<String> {
    let domain = redis_store::get_tab_domain(session_id, tab_id)?;
    let intent = redis_store::get_intent(session_id, tab_id)?;

    let all_known = qdrant_store::known_stages(&domain, &intent);
    let mut done_names: std::collections::HashSet<String> =
        redis_store::get_done_stages(session_id, tab_id).into_iter().collect();

    // Step 1: has anything NOT already done just become true on this page?
    for stage in &all_known {
        if done_names.contains(&stage.stage) {
            continue;
        }
        if condition_match::page_satisfies_condition(page_text, &stage.post_condition) >= PAGE_DONE_THRESHOLD {
            redis_store::mark_stage_done(session_id, tab_id, &stage.stage);
            done_names.insert(stage.stage.clone());
        }
    }

    // Cold start: nothing done yet, even after the check above - no confident way to
    // pick a first stage (see hint.rs's design discussion), so stay quiet.
    if done_names.is_empty() {
        return None;
    }

    let done_post_conditions: Vec<String> = all_known
        .iter()
        .filter(|s| done_names.contains(&s.stage))
        .map(|s| s.post_condition.clone())
        .collect();

    // Step 2: of what's left, which one follows on best from what's already done -
    // cross-site candidate pool, same as select_stage (see candidate_stages' docs).
    let mut scored: Vec<(f32, qdrant_store::ScoredStage)> = qdrant_store::candidate_stages(&intent)
        .into_iter()
        .filter(|c| !done_names.contains(&c.stage))
        .map(|c| {
            let cond_score = done_post_conditions
                .iter()
                .map(|post| condition_match::word_overlap(&c.pre_condition, post))
                .fold(0.0_f32, f32::max);
            let score = (1.0 - STAGE_COND_WEIGHT) * c.semantic_score + STAGE_COND_WEIGHT * cond_score;
            (score, c)
        })
        .collect();

    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let (top_score, top) = &scored[0];
    let runner_up_score = scored.get(1).map(|(s, _)| *s).unwrap_or(*top_score);
    if *top_score < STAGE_MIN_CONFIDENCE || (scored.len() > 1 && top_score - runner_up_score < STAGE_MIN_MARGIN) {
        return None;
    }

    Some(render_next_stage(top))
}

fn render_workflow(domain: &str, workflow: &qdrant_store::MatchedWorkflow) -> String {
    let mut text = format!(
        "Known workflow for \"{}\" on {domain} (match score {:.2}):\n",
        workflow.description, workflow.score,
    );
    for (i, (key, value)) in workflow.steps.iter().enumerate() {
        text.push_str(&format!("{}. {key} = {value}\n", i + 1));
    }
    text.push_str(BATCH_TIP);
    text
}

// Appended wherever a hint lists multiple known actions: recall knows the sequence but
// not the live refs/coordinates for this page load, so it can't batch the calls itself -
// this just tells the caller (Claude) to do so once it has them, instead of one
// tool-call-and-response round trip per action. Verified this matters: in this session's
// own live demo runs, action-by-action execution (click, then type, then key) was the
// single biggest source of avoidable tool calls, bigger than any hint content itself.
const BATCH_TIP: &str = "Once you have the real refs for these, execute them together in \
a single browser_batch call rather than one tool call per action - saves a full \
round trip per step.\n";

fn render_next_stage(stage: &qdrant_store::ScoredStage) -> String {
    format!(
        "Next step: {} (on {}) - starts when: {}; ends when: {}\n",
        stage.stage, stage.domain, stage.pre_condition, stage.post_condition,
    )
}

fn render_plan(domain: &str, intent: &str, stages: &[qdrant_store::KnownStage]) -> String {
    let mut text = format!("Known plan for \"{intent}\" on {domain} (most-used stages first):\n");
    for (i, stage) in stages.iter().enumerate() {
        text.push_str(&format!(
            "{}. {} [seen {}x] - starts when: {}; ends when: {}\n",
            i + 1,
            stage.stage,
            stage.count,
            stage.pre_condition,
            stage.post_condition,
        ));
    }
    text.push_str(BATCH_TIP);
    text
}
