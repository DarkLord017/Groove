use redis::Commands;
use shared::PageElement;

const ELEMENTS_TTL_SECONDS: usize = 30 * 60;
const FOCUS_TTL_SECONDS: usize = 60 * 60;

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
