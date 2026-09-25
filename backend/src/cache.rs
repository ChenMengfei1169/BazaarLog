// Tiny in-memory TTL cache used as a zero-dependency fallback for Redis. It
// stores serialized JSON payloads keyed by semantic cache keys. Mutations
// invalidate the relevant keys so stale dashboards are never served.
//
// Entries carry their own expiry rather than sharing one global TTL, so a
// security-sensitive value such as the audit chain report can be cached for
// much less time than a dashboard aggregate.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Cache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    default_ttl: Duration,
}

/// One cached payload together with the instant it stops being served.
struct CacheEntry {
    expires_at: Instant,
    payload: String,
}

impl Cache {
    /// Creates an empty cache whose entries live for `ttl_secs` seconds unless
    /// a caller passes an explicit TTL to [`Cache::set_with_ttl`].
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            default_ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Returns the payload cached under `key`, or None when it is absent or
    /// expired. An expired entry is dropped on the way out.
    pub fn get(&self, key: &str) -> Option<String> {
        let mut entries = self.entries.lock().ok()?;
        let entry = entries.get(key)?;
        if Instant::now() < entry.expires_at {
            return Some(entry.payload.clone());
        }
        entries.remove(key);
        None
    }

    /// Stores a value under the cache's default TTL.
    pub fn set(&self, key: String, value: String) {
        self.set_with_ttl(key, value, self.default_ttl);
    }

    /// Stores a value under an explicit TTL, for callers whose staleness budget
    /// differs from the default.
    pub fn set_with_ttl(&self, key: String, value: String, ttl: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key,
                CacheEntry {
                    expires_at: Instant::now() + ttl,
                    payload: value,
                },
            );
        }
    }

    /// Drops a cached entry so the next read rebuilds it.
    pub fn invalidate(&self, key: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }
}
