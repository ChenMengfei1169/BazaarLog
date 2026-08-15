// Tiny in-memory TTL cache used as a zero-dependency fallback for Redis. It
// stores serialized JSON payloads keyed by semantic cache keys. Mutations
// invalidate the relevant keys so stale dashboards are never served.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Cache {
    inner: Mutex<HashMap<String, (Instant, String)>>,
    ttl: Duration,
}

impl Cache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let mut map = self.inner.lock().ok()?;
        if let Some((ts, val)) = map.get(key) {
            if ts.elapsed() < self.ttl {
                return Some(val.clone());
            }
            map.remove(key);
        }
        None
    }

    pub fn set(&self, key: String, val: String) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(key, (Instant::now(), val));
        }
    }

    pub fn invalidate(&self, key: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(key);
        }
    }
}
