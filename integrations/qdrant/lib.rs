use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use qdrant_client::qdrant::{Condition, Filter, QueryPointsBuilder, ScrollPointsBuilder};
use qdrant_client::{Payload, Qdrant};

// Same collections, same embedding model (sentence-transformers/all-MiniLM-L6-v2, 384-dim,
// cosine) as learn/src/learn/hmt/site_tree.py - recall only ever reads what learn writes,
// so the vector space and collection names must match exactly or similarity is meaningless.
const INTENTS_COLLECTION: &str = "site_intents";
const STAGES_COLLECTION: &str = "site_stages";
const WORKFLOWS_COLLECTION: &str = "site_workflows";
const FACTS_COLLECTION: &str = "user_facts";

// Same floor key_match.py uses for its bi-encoder stage (MIN_MINILM_FLOOR) - below this a
// cosine match is noise, not a real match. There's no cross-encoder rerank on this path (the
// diagram's "Intent match" step is pure embeddings), so this floor is what stands in for it.
const INTENT_MATCH_FLOOR: f32 = 0.30;
const INTENT_MATCH_MARGIN: f32 = 0.1;

// Verified on real data: benchmarks/awm_intent_match_benchmark.py. The floor alone let a
// genuinely unrelated prompt ("what's the weather like today") false-match a workflow at
// 0.331 - its margin over the runner-up was only 0.029, versus 0.23-0.54 for every real
// match. Floor + margin together scored 8/8; floor alone scored 7/8. Same MIN_MARGIN
// site_tree.py's select_stage already uses for the same reason.
const WORKFLOW_MATCH_FLOOR: f32 = 0.30;
const WORKFLOW_MATCH_MARGIN: f32 = 0.1;

// Same floor+margin shape as workflow matching - structurally the same problem (one
// short label matched against a handful of other short labels via embeddings only), so
// the same thresholds apply rather than inventing new ones.
const FACT_MATCH_FLOOR: f32 = 0.30;
const FACT_MATCH_MARGIN: f32 = 0.1;

fn qdrant_url() -> String {
    std::env::var("QDRANT_GRPC_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_string())
}

fn client() -> &'static Qdrant {
    static CLIENT: OnceLock<Qdrant> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Qdrant::from_url(&qdrant_url())
            .build()
            .expect("qdrant_store: failed to build Qdrant client")
    })
}

// fastembed's TextEmbedding is a synchronous, local ONNX model (no network call per
// embed() - only the first load downloads/caches weights) - a plain Mutex<OnceLock> is
// enough, no need to route this through the tokio runtime below.
// fastembed defaults to caching model weights under "./.fastembed_cache" (cwd-relative) -
// fine for a long-lived server, but recall runs once per hook call with cwd set to
// whatever project Claude Code is working in, so that default would litter every such
// project with a 90+MB cache dir. Pin it to a stable, shared location instead.
fn cache_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("FASTEMBED_CACHE_DIR") {
        return dir.into();
    }
    let base = dirs_cache_base();
    base.join("groove").join("fastembed")
}

fn dirs_cache_base() -> std::path::PathBuf {
    std::env::var("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("qdrant_store: HOME is not set");
            std::path::PathBuf::from(home).join(".cache")
        })
}

fn model() -> &'static Mutex<TextEmbedding> {
    static MODEL: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();
    MODEL.get_or_init(|| {
        let model = TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::AllMiniLML6V2).with_cache_dir(cache_dir()),
        )
        .expect("qdrant_store: failed to load the AllMiniLML6V2 embedding model");
        Mutex::new(model)
    })
}

fn embed(text: &str) -> Vec<f32> {
    let mut model = model().lock().expect("qdrant_store: embedding model mutex poisoned");
    let mut embeddings = model
        .embed(vec![text], None)
        .expect("qdrant_store: embed() failed");
    embeddings.remove(0)
}

// recall's main() is synchronous (stdin -> stdout, one hook call per process) and the
// qdrant-client crate is async-only - a single current-thread runtime, built once and
// reused via block_on, keeps every call site here a plain blocking function. Mirrors how
// redis_store/rabbitmq already present a synchronous API to their callers.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("qdrant_store: failed to build tokio runtime")
    })
}

fn payload_str(payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.clone().into_json().as_str().map(str::to_string))
}

/// A top match is trusted when it clears the floor, and - when a real runner-up exists
/// to compare against - beats it by the margin too. With no runner-up at all (the only
/// candidate in the whole collection), the floor alone decides: by explicit call, a lone
/// match above the floor is accepted as-is, understanding that without a second
/// candidate to sanity-check against this can occasionally be wrong (verified live:
/// e.g. an unrelated prompt false-matching a domain's only learned intent at 0.33-0.41 -
/// a real, accepted risk now, not an unnoticed gap).
fn clears_floor_and_margin(top_score: f32, runner_up_score: Option<f32>, floor: f32, margin: f32) -> bool {
    if top_score < floor {
        return false;
    }
    match runner_up_score {
        Some(runner_up) => top_score - runner_up >= margin,
        None => true,
    }
}

#[derive(Debug, Clone)]
pub struct KnownStage {
    pub stage: String,
    pub pre_condition: String,
    pub post_condition: String,
    pub count: i64,
}

#[derive(Debug, Clone)]
pub struct MatchedWorkflow {
    pub description: String,
    pub steps: Vec<(String, String)>,
    pub score: f32,
}

fn site_known(collection: &str, domain: &str) -> bool {
    let request = ScrollPointsBuilder::new(collection)
        .filter(Filter::all([Condition::matches("domain", domain.to_string())]))
        .limit(1)
        .with_payload(false)
        .with_vectors(false);

    let result = runtime().block_on(client().scroll(request));
    match result {
        Ok(response) => !response.result.is_empty(),
        Err(err) => {
            eprintln!("qdrant_store: site_known({collection}) scroll failed: {err}");
            false
        }
    }
}

/// Is there any AWM workflow at all for this domain. No widening (site -> platform ->
/// category): explicitly scoped out, so a miss here just means "nothing to hint."
pub fn awm_known_site(domain: &str) -> bool {
    site_known(WORKFLOWS_COLLECTION, domain)
}

/// Is there any HMT stage data at all for this domain - the fallback path's own gate,
/// independent of AWM's (a domain can be known to one, both, or neither).
pub fn hmt_known_site(domain: &str) -> bool {
    site_known(STAGES_COLLECTION, domain)
}

/// Matches a live prompt against this domain's known AWM workflow descriptions - pure
/// embedding similarity, no LLM (recall runs inside a synchronous hook). Query text is
/// domain-prefixed ("{domain}: {prompt}") to match how workflow.py embeds stored
/// descriptions (unlike site_stages, which deliberately excludes domain - see
/// known_stages). Floor + margin both required: verified on real data
/// (benchmarks/awm_intent_match_benchmark.py) that the floor alone isn't enough to
/// reject a confidently-wrong top-1 when nothing actually matches.
pub fn match_workflow(domain: &str, prompt: &str) -> Option<MatchedWorkflow> {
    let query_vector = embed(&format!("{domain}: {prompt}"));
    let request = QueryPointsBuilder::new(WORKFLOWS_COLLECTION)
        .query(query_vector)
        .filter(Filter::all([Condition::matches("domain", domain.to_string())]))
        .limit(2)
        .with_payload(true);

    let result = runtime().block_on(client().query(request));
    let hits = match result {
        Ok(response) => response.result,
        Err(err) => {
            eprintln!("qdrant_store: match_workflow query failed: {err}");
            return None;
        }
    };

    let top = hits.first()?;
    let runner_up_score = hits.get(1).map(|h| h.score);
    if !clears_floor_and_margin(top.score, runner_up_score, WORKFLOW_MATCH_FLOOR, WORKFLOW_MATCH_MARGIN) {
        return None;
    }

    let payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
        Payload::from(top.payload.clone()).into();
    let description = payload_str(&payload, "description")?;
    let steps = latest_workflow_steps(&payload);

    Some(MatchedWorkflow { description, steps, score: top.score })
}

#[derive(Debug, Clone)]
pub struct MatchedFact {
    pub key: String,
    pub value: String,
    pub score: f32,
}

/// Matches a form field's resolved label (e.g. `textbox "Home city"` - exactly what
/// redis_store::lookup_element returns) against saved facts, no LLM, same embedding +
/// floor + margin approach as match_workflow. No domain filter: facts.py's user_facts
/// collection is global to the user, not per-site (a home city is the same everywhere).
/// Fact keys are stored in this exact "role \"label\"" format (see field_pairs.py's
/// _save_pair, which passes the already-resolved ref straight through as the key), so
/// the query text needs no stripping/reformatting to line up with what's stored.
pub fn match_fact(field_label: &str) -> Option<MatchedFact> {
    let query_vector = embed(field_label);
    let request = QueryPointsBuilder::new(FACTS_COLLECTION)
        .query(query_vector)
        .limit(2)
        .with_payload(true);

    let result = runtime().block_on(client().query(request));
    let hits = match result {
        Ok(response) => response.result,
        Err(err) => {
            eprintln!("qdrant_store: match_fact query failed: {err}");
            return None;
        }
    };

    let top = hits.first()?;
    let runner_up_score = hits.get(1).map(|h| h.score);
    if !clears_floor_and_margin(top.score, runner_up_score, FACT_MATCH_FLOOR, FACT_MATCH_MARGIN) {
        return None;
    }

    let payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
        Payload::from(top.payload.clone()).into();
    let key = payload_str(&payload, "key")?;
    let value = latest_fact_value(&payload)?;

    Some(MatchedFact { key, value, score: top.score })
}

fn latest_fact_value(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
) -> Option<String> {
    let values = payload.get("values").map(|v| v.clone().into_json()).and_then(|v| v.as_array().cloned())?;
    values.last()?.get("value")?.as_str().map(str::to_string)
}

fn latest_workflow_steps(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
) -> Vec<(String, String)> {
    let history = payload
        .get("steps_history")
        .map(|v| v.clone().into_json())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let Some(latest) = history.last() else { return Vec::new() };
    let steps = latest.get("steps").and_then(|s| s.as_array()).cloned().unwrap_or_default();

    steps
        .into_iter()
        .filter_map(|s| {
            let key = s.get("key")?.as_str()?.to_string();
            let value = s.get("value")?.as_str()?.to_string();
            Some((key, value))
        })
        .collect()
}

/// "Intent match" - embeds `objective` and finds the closest previously-resolved intent
/// for this domain (top-1 of the diagram's "top 5", since a single best match is all a
/// hint needs). Read-only: unlike learn's _resolve_intent, this never upserts a new
/// intent point - recall only reads what learn has already written.
pub fn match_intent(domain: &str, objective: &str) -> Option<String> {
    let query_vector = embed(objective);
    let request = QueryPointsBuilder::new(INTENTS_COLLECTION)
        .query(query_vector)
        .filter(Filter::all([Condition::matches("domain", domain.to_string())]))
        .limit(2)
        .with_payload(true);

    let result = runtime().block_on(client().query(request));
    let hits = match result {
        Ok(response) => response.result,
        Err(err) => {
            eprintln!("qdrant_store: match_intent query failed: {err}");
            return None;
        }
    };

    let top = hits.first()?;
    let runner_up_score = hits.get(1).map(|h| h.score);
    if !clears_floor_and_margin(top.score, runner_up_score, INTENT_MATCH_FLOOR, INTENT_MATCH_MARGIN) {
        return None;
    }
    payload_str(&top.payload, "intent")
}

/// Every known stage for this domain+intent, most-occurred first - the "plan" surfaced on
/// navigate: a readable summary of what's usually done here, not a single next-step pick
/// (that needs done-stage tracking, which doesn't exist yet - see the diagram's "Redis -
/// done stages" box).
pub fn known_stages(domain: &str, intent: &str) -> Vec<KnownStage> {
    let request = ScrollPointsBuilder::new(STAGES_COLLECTION)
        .filter(Filter::all([
            Condition::matches("domain", domain.to_string()),
            Condition::matches("intent", intent.to_string()),
        ]))
        .limit(10_000)
        .with_payload(true)
        .with_vectors(false);

    let result = runtime().block_on(client().scroll(request));
    let points = match result {
        Ok(response) => response.result,
        Err(err) => {
            eprintln!("qdrant_store: known_stages scroll failed: {err}");
            return Vec::new();
        }
    };

    let mut stages: Vec<KnownStage> = points
        .into_iter()
        .filter_map(|p| {
            let payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
                Payload::from(p.payload).into();
            Some(KnownStage {
                stage: payload_str(&payload, "stage")?,
                pre_condition: payload_str(&payload, "pre_condition").unwrap_or_default(),
                post_condition: payload_str(&payload, "post_condition").unwrap_or_default(),
                count: payload
                    .get("count")
                    .and_then(|v| v.clone().into_json().as_i64())
                    .unwrap_or(0),
            })
        })
        .collect();

    stages.sort_by(|a, b| b.count.cmp(&a.count));
    stages
}

// Same K_G as site_tree.py's select_stage - how many nearest-by-meaning candidates to
// pull before any sequencing logic narrows them down.
const STAGE_CANDIDATES_K: u64 = 8;

#[derive(Debug, Clone)]
pub struct ScoredStage {
    pub domain: String,
    pub stage: String,
    pub pre_condition: String,
    pub post_condition: String,
    /// Raw semantic similarity to the intent (recall's "Tool 1") - combining this with
    /// a sequencing score (recall's "Tool 2", computed in recall itself from these
    /// conditions - no vector math needed there) is the caller's job, same split as
    /// site_tree.py's select_stage.
    pub semantic_score: f32,
}

/// The next-stage picker's candidate pool: every stage near this intent in meaning,
/// across ALL domains - no domain filter, deliberately mirroring select_stage's own
/// (already-shipped, documented) choice to generalize across sites rather than stay
/// domain-scoped, unlike known_stages/awm matching which are domain-scoped on purpose.
pub fn candidate_stages(intent: &str) -> Vec<ScoredStage> {
    let query_vector = embed(intent);
    let request = QueryPointsBuilder::new(STAGES_COLLECTION)
        .query(query_vector)
        .filter(Filter::all([Condition::matches("intent", intent.to_string())]))
        .limit(STAGE_CANDIDATES_K)
        .with_payload(true);

    let result = runtime().block_on(client().query(request));
    let hits = match result {
        Ok(response) => response.result,
        Err(err) => {
            eprintln!("qdrant_store: candidate_stages query failed: {err}");
            return Vec::new();
        }
    };

    hits.into_iter()
        .filter_map(|hit| {
            let payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
                Payload::from(hit.payload).into();
            Some(ScoredStage {
                domain: payload_str(&payload, "domain").unwrap_or_default(),
                stage: payload_str(&payload, "stage")?,
                pre_condition: payload_str(&payload, "pre_condition").unwrap_or_default(),
                post_condition: payload_str(&payload, "post_condition").unwrap_or_default(),
                semantic_score: hit.score,
            })
        })
        .collect()
}
