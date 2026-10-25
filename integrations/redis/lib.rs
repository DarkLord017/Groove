use redis::Commands;
use shared::PageElement;

const ELEMENTS_TTL_SECONDS: usize = 30 * 60;
const FOCUS_TTL_SECONDS: usize = 60 * 60;
const PROMPTS_TTL_SECONDS: usize = 60 * 60;
const INTENT_TTL_SECONDS: usize = 60 * 60;
const DONE_STAGES_TTL_SECONDS: usize = 60 * 60;

// Kept one bigger than the largest window anything reads (6) so a reader can still see
// a full window's worth of prompts even if the background consumer is a message behind.
const PROMPTS_BUFFER_SIZE: isize = 8;

fn connection_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())
}

fn connect() -> Result<redis::Connection, redis::RedisError> {
    let client = redis::Client::open(connection_url())?;
    client.get_connection()
}

fn elements_key(session_id: &str, tab_id: Option<f64>) -> String {
    match tab_id {
        Some(tab_id) => format!("elements:{session_id}:{tab_id}"),
        None => format!("elements:{session_id}"),
    }
}

fn focus_key(session_id: &str, tab_id: f64) -> String {
    format!("focus:{session_id}:{tab_id}")
}

fn prompts_key(session_id: &str) -> String {
    format!("prompts:{session_id}")
}

// Keyed by (session_id, tab_id) - same as elements/focus - a session can have several
// tabs open on different sites/tasks at once, so the resolved intent for one tab must
// not bleed into another.
fn intent_key(session_id: &str, tab_id: f64) -> String {
    format!("intent:{session_id}:{tab_id}")
}

// Paired with intent_key - done-stage tracking needs the domain too (to look up this
// tab's known stages), and domain/intent are always resolved and cached together on
// navigate, so this is just intent_key's sibling, not a reuse of learn's own
// session-scoped "site:domain:{session_id}" key (that one isn't tab-scoped).
fn tab_domain_key(session_id: &str, tab_id: f64) -> String {
    format!("tab_domain:{session_id}:{tab_id}")
}

fn done_stages_key(session_id: &str, tab_id: f64) -> String {
    format!("done_stages:{session_id}:{tab_id}")
}

pub fn cache_elements(session_id: &str, tab_id: Option<f64>, elements: &[PageElement]) {
    if elements.is_empty() {
        return;
    }

    let key = elements_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        let pairs: Vec<(&str, &str)> = elements
            .iter()
            .map(|element| (element.ref_id.as_str(), element.description.as_str()))
            .collect();
        conn.hset_multiple::<_, _, _, ()>(&key, &pairs)?;
        conn.expire::<_, ()>(&key, ELEMENTS_TTL_SECONDS as i64)?;
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("capture: redis cache_elements failed for {key}: {err}");
    }
}

pub fn lookup_element(session_id: &str, tab_id: f64, ref_id: &str) -> Option<String> {
    let key = elements_key(session_id, Some(tab_id));
    let result: redis::RedisResult<Option<String>> = (|| {
        let mut conn = connect()?;
        conn.hget(&key, ref_id)
    })();

    match result {
        Ok(value) => value,
        Err(err) => {
            eprintln!("capture: redis lookup_element failed for {key}: {err}");
            None
        }
    }
}

pub fn set_focus(session_id: &str, tab_id: f64, ref_id: &str) {
    let key = focus_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.set_ex(&key, ref_id, FOCUS_TTL_SECONDS as u64)
    })();

    if let Err(err) = result {
        eprintln!("capture: redis set_focus failed for {key}: {err}");
    }
}

pub fn get_focus(session_id: &str, tab_id: f64) -> Option<String> {
    let key = focus_key(session_id, tab_id);
    let result: redis::RedisResult<Option<String>> = (|| {
        let mut conn = connect()?;
        conn.get(&key)
    })();

    match result {
        Ok(value) => value,
        Err(err) => {
            eprintln!("capture: redis get_focus failed for {key}: {err}");
            None
        }
    }
}

pub fn delete_tab_maps(session_id: &str, tab_id: f64) {
    let elements = elements_key(session_id, Some(tab_id));
    let focus = focus_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        let _: () = conn.del(&elements)?;
        let _: () = conn.del(&focus)?;
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("capture: redis delete_tab_maps failed for {elements}/{focus}: {err}");
    }
}

pub fn save_prompt(session_id: &str, prompt: &str) {
    let key = prompts_key(session_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.lpush::<_, _, ()>(&key, prompt)?;
        conn.ltrim::<_, ()>(&key, 0, PROMPTS_BUFFER_SIZE - 1)?;
        conn.expire::<_, ()>(&key, PROMPTS_TTL_SECONDS as i64)?;
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("recall: redis save_prompt failed for {key}: {err}");
    }
}

// Returns up to `count` most recent prompts, newest first. Callers asking for the usual
// 5-6 message window still get it even though the buffer stores a couple extra - that
// slack is there for a background consumer that hasn't caught up yet, not for readers.
pub fn get_recent_prompts(session_id: &str, count: isize) -> Vec<String> {
    let key = prompts_key(session_id);
    let result: redis::RedisResult<Vec<String>> = (|| {
        let mut conn = connect()?;
        conn.lrange(&key, 0, count - 1)
    })();

    match result {
        Ok(prompts) => prompts,
        Err(err) => {
            eprintln!("recall: redis get_recent_prompts failed for {key}: {err}");
            Vec::new()
        }
    }
}

/// Caches the resolved HMT intent for this (session, tab) - set on every navigate,
/// regardless of whether AWM also matched a workflow for the same prompt, so later
/// PostToolUse calls on this tab can cheaply read "what task is this" without
/// re-resolving it (and so HMT has something to fall back on if an AWM workflow runs out
/// of steps mid-task).
pub fn set_intent(session_id: &str, tab_id: f64, intent: &str) {
    let key = intent_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.set_ex(&key, intent, INTENT_TTL_SECONDS as u64)
    })();

    if let Err(err) = result {
        eprintln!("recall: redis set_intent failed for {key}: {err}");
    }
}

pub fn get_intent(session_id: &str, tab_id: f64) -> Option<String> {
    let key = intent_key(session_id, tab_id);
    let result: redis::RedisResult<Option<String>> = (|| {
        let mut conn = connect()?;
        conn.get(&key)
    })();

    match result {
        Ok(value) => value,
        Err(err) => {
            eprintln!("recall: redis get_intent failed for {key}: {err}");
            None
        }
    }
}

/// Clears a stale intent rather than leaving it to sit under its TTL - called when a
/// fresh navigate's own intent resolution comes back empty, so a later PostToolUse call
/// on this tab reads "nothing known" instead of a previous task's leftover answer.
pub fn clear_intent(session_id: &str, tab_id: f64) {
    let key = intent_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.del(&key)
    })();

    if let Err(err) = result {
        eprintln!("recall: redis clear_intent failed for {key}: {err}");
    }
}

/// Caches the domain alongside the intent for this (session, tab) - set together with
/// set_intent on every navigate, read together by the PostToolUse stage tracker, which
/// needs both to look up this tab's known stages.
pub fn set_tab_domain(session_id: &str, tab_id: f64, domain: &str) {
    let key = tab_domain_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.set_ex(&key, domain, INTENT_TTL_SECONDS as u64)
    })();

    if let Err(err) = result {
        eprintln!("recall: redis set_tab_domain failed for {key}: {err}");
    }
}

pub fn get_tab_domain(session_id: &str, tab_id: f64) -> Option<String> {
    let key = tab_domain_key(session_id, tab_id);
    let result: redis::RedisResult<Option<String>> = (|| {
        let mut conn = connect()?;
        conn.get(&key)
    })();

    match result {
        Ok(value) => value,
        Err(err) => {
            eprintln!("recall: redis get_tab_domain failed for {key}: {err}");
            None
        }
    }
}

/// Adds a stage to this (session, tab)'s done set - called once the page-text check
/// decides a stage's post_condition has become true. A Redis SET, not a list: a stage
/// either is or isn't done, no ordering or duplicates to track.
pub fn mark_stage_done(session_id: &str, tab_id: f64, stage: &str) {
    let key = done_stages_key(session_id, tab_id);
    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        conn.sadd::<_, _, ()>(&key, stage)?;
        conn.expire::<_, ()>(&key, DONE_STAGES_TTL_SECONDS as i64)?;
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("recall: redis mark_stage_done failed for {key}: {err}");
    }
}

pub fn get_done_stages(session_id: &str, tab_id: f64) -> Vec<String> {
    let key = done_stages_key(session_id, tab_id);
    let result: redis::RedisResult<Vec<String>> = (|| {
        let mut conn = connect()?;
        conn.smembers(&key)
    })();

    match result {
        Ok(stages) => stages,
        Err(err) => {
            eprintln!("recall: redis get_done_stages failed for {key}: {err}");
            Vec::new()
        }
    }
}

/// Resets every tab's run state for this session - called on Stop (turn complete), so
/// the next run starts with a clean slate instead of inheriting a previous task's done
/// stages or cached intent/domain. Stop carries no tab_id (it's a whole-turn event, not
/// a per-tool-call one), so this scans by session prefix rather than taking one - same
/// pattern delete_session_maps already uses for the same reason.
pub fn clear_run_state(session_id: &str) {
    let patterns = [
        format!("intent:{session_id}:*"),
        format!("tab_domain:{session_id}:*"),
        format!("done_stages:{session_id}:*"),
    ];

    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        for pattern in &patterns {
            let keys: Vec<String> = conn.scan_match(pattern)?.collect();
            if !keys.is_empty() {
                let _: () = conn.del(&keys)?;
            }
        }
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("recall: redis clear_run_state failed for session {session_id}: {err}");
    }
}

// Redis has no native prefix-delete, so this scans for keys matching each pattern and
// deletes whatever turns up. SCAN (not KEYS) so a session with many tabs doesn't block
// the server while iterating.
pub fn delete_session_maps(session_id: &str) {
    let patterns = [
        format!("elements:{session_id}:*"),
        format!("elements:{session_id}"),
        format!("focus:{session_id}:*"),
    ];

    let result: redis::RedisResult<()> = (|| {
        let mut conn = connect()?;
        for pattern in &patterns {
            let keys: Vec<String> = conn.scan_match(pattern)?.collect();
            if !keys.is_empty() {
                let _: () = conn.del(&keys)?;
            }
        }
        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("capture: redis delete_session_maps failed for session {session_id}: {err}");
    }
}
