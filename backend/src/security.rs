// Request hardening: defensive response headers, per-key rate limiting, and a
// global per-class failed-login tracker.
//
// The rate limiters are sliding-window buckets keyed by strings. The keys are
// built from the client address plus the class id where one is known, so
// rotating source addresses cannot cheaply reset a bucket that is bound to a
// class. The global limiter blunts generic traffic abuse and denial of service;
// the stricter limiter protects the password-checking endpoints from brute
// forcing; the class-auth limiter throttles every request that carries class
// credentials, each of which runs an Argon2 verification.
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// Sliding-window rate limiter keyed by an arbitrary string. The bucket map is
/// deliberately bounded so a flood of requests from many distinct keys cannot
/// grow memory without limit.
#[derive(Clone)]
pub struct RateLimiter {
    shared: Arc<Mutex<RateLimiterState>>,
    max_requests: usize,
    window_duration: Duration,
    // Upper bound on tracked keys. Protects against memory exhaustion when an
    // attacker rotates many distinct addresses or class ids.
    max_tracked_keys: usize,
}

// Shared mutable state behind the limiter's mutex: the per-key sliding-window
// queues plus the timestamp of the most recent bucket eviction.
struct RateLimiterState {
    buckets: HashMap<String, VecDeque<Instant>>,
    last_eviction: Option<Instant>,
}

// Comfortably above any realistic single-host deployment while still bounding
// memory; each entry holds at most `max_requests` timestamps.
const MAX_TRACKED_KEYS: usize = 100_000;

/// True when `timestamp` has fallen outside a sliding window of
/// `window_duration` that ends at `now`. Measured as an elapsed-time difference
/// rather than as `now - window_duration`, because subtracting a Duration from
/// an Instant can underflow and panic while `saturating_duration_since` cannot.
fn has_fallen_out_of_window(now: Instant, timestamp: Instant, window_duration: Duration) -> bool {
    now.saturating_duration_since(timestamp) > window_duration
}

impl RateLimiter {
    pub fn new(max_requests: usize, window_seconds: u64) -> Self {
        Self {
            shared: Arc::new(Mutex::new(RateLimiterState {
                buckets: HashMap::new(),
                last_eviction: None,
            })),
            max_requests,
            window_duration: Duration::from_secs(window_seconds),
            max_tracked_keys: MAX_TRACKED_KEYS,
        }
    }

    /// Returns true when `key` still has budget inside the sliding window.
    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = shared.buckets.entry(key.to_string()).or_default();
        // Drop timestamps that have fallen out of the sliding window.
        while queue.front().is_some_and(|timestamp| {
            has_fallen_out_of_window(now, *timestamp, self.window_duration)
        }) {
            queue.pop_front();
        }
        if queue.len() >= self.max_requests {
            return false;
        }
        queue.push_back(now);
        // Bound memory. The eviction sweep runs at most once per window so a
        // flood of distinct keys cannot turn every request into an O(n log n)
        // pass while holding the lock.
        if shared.buckets.len() > self.max_tracked_keys
            && shared.last_eviction.map_or(true, |evicted_at| {
                has_fallen_out_of_window(now, evicted_at, self.window_duration)
            })
        {
            shared.last_eviction = Some(now);
            shared.buckets.retain(|_, timestamps| {
                timestamps.back().is_some_and(|latest| {
                    !has_fallen_out_of_window(now, *latest, self.window_duration)
                })
            });
            // Still over the cap? Evict the least-recently-active buckets in a
            // single pass so the eviction cost stays O(n log n) per sweep.
            let excess = shared.buckets.len().saturating_sub(self.max_tracked_keys);
            if excess > 0 {
                // Collect owned keys and timestamps, both Clone, so the map can
                // be mutated while evicting without holding borrows into it.
                let mut by_recency: Vec<(String, Instant)> = shared
                    .buckets
                    .iter()
                    .filter_map(|(key, timestamps)| {
                        timestamps.back().map(|latest| (key.clone(), *latest))
                    })
                    .collect();
                by_recency.sort_by_key(|(_, latest)| *latest);
                for (key, _) in by_recency.into_iter().take(excess) {
                    shared.buckets.remove(&key);
                }
            }
        }
        true
    }
}

/// Global per-class failed-login counter. Independent of source address, so an
/// attacker who rotates addresses still trips it: once a class has recorded
/// `max_failures` failed verifications inside the window, further attempts are
/// rejected until failures slide out of the window or a login succeeds.
#[derive(Clone)]
pub struct AuthFailureTracker {
    failures: Arc<Mutex<HashMap<i64, VecDeque<Instant>>>>,
    max_failures: usize,
    window_duration: Duration,
    max_tracked_classes: usize,
}

// 10 failures per class per 5 minutes is generous for a classroom, where a
// teacher may mistype a password, while capping scripted brute force hard, even
// across thousands of rotating source addresses.
const MAX_FAILURES_PER_WINDOW: usize = 10;
const FAILURE_WINDOW_SECS: u64 = 300;
const MAX_TRACKED_CLASSES: usize = 2_000;

impl AuthFailureTracker {
    pub fn new() -> Self {
        Self {
            failures: Arc::new(Mutex::new(HashMap::new())),
            max_failures: MAX_FAILURES_PER_WINDOW,
            window_duration: Duration::from_secs(FAILURE_WINDOW_SECS),
            max_tracked_classes: MAX_TRACKED_CLASSES,
        }
    }

    /// True when the class has exhausted its failure budget in the window.
    pub fn is_blocked(&self, class_id: i64) -> bool {
        let now = Instant::now();
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = failures.entry(class_id).or_default();
        while queue.front().is_some_and(|timestamp| {
            has_fallen_out_of_window(now, *timestamp, self.window_duration)
        }) {
            queue.pop_front();
        }
        queue.len() >= self.max_failures
    }

    /// Records one failed verification for the class.
    pub fn record_failure(&self, class_id: i64) {
        let now = Instant::now();
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = failures.entry(class_id).or_default();
        while queue.front().is_some_and(|timestamp| {
            has_fallen_out_of_window(now, *timestamp, self.window_duration)
        }) {
            queue.pop_front();
        }
        queue.push_back(now);
        // Bound memory for a flood of distinct class ids.
        if failures.len() > self.max_tracked_classes {
            failures.retain(|_, timestamps| {
                timestamps.back().is_some_and(|latest| {
                    !has_fallen_out_of_window(now, *latest, self.window_duration)
                })
            });
        }
    }

    /// Clears the failure history for a class after a successful login.
    pub fn record_success(&self, class_id: i64) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(&class_id);
        }
    }
}

// The password-checking endpoints deserve a stricter budget than the rest of
// the API. Only write requests are considered so the public class list stays on
// the global budget.
fn is_authentication_request(method: &Method, path: &str) -> bool {
    if method != Method::POST {
        return false;
    }
    path == "/api/classes" || (path.starts_with("/api/classes/") && path.ends_with("/auth"))
}

// Chain verification reads the whole audit table and recomputes a SHA-256 per
// row, so it is far more expensive than an ordinary read and gets its own, much
// tighter budget instead of sharing the generic one.
fn is_audit_chain_request(method: &Method, path: &str) -> bool {
    method == Method::GET
        && path.starts_with("/api/classes/")
        && path.ends_with("/audit_logs/chain")
}

// Requests carrying class credentials run an Argon2 verification in the
// ClassAuth extractor. Matching on the class id header too is important: a
// request with x-class-id but no x-class-password still verifies the empty
// password against the stored hash, so it must consume the same tighter budget
// or an attacker could burn server CPU at twice the intended rate.
fn carries_class_credentials(request: &Request) -> bool {
    request.headers().contains_key("x-class-id")
        || request.headers().contains_key("x-class-password")
}

// Extracts the class id from /api/classes/:id/auth or from the x-class-id
// header, so limiter keys can be bound to the class in addition to the address.
fn request_class_key(request: &Request, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("/api/classes/") {
        if let Some(id_part) = rest.strip_suffix("/auth") {
            if let Ok(class_id) = id_part.parse::<i64>() {
                return class_id.to_string();
            }
        }
    }
    request
        .headers()
        .get("x-class-id")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// The client address of a request, or the unspecified address when the
/// connection info extension is absent.
fn request_client_ip(request: &Request) -> IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |connect_info| {
            connect_info.0.ip()
        })
}

/// Explicit CSRF hardening: browsers attach an Origin header to state-changing
/// cross-origin requests. Rejecting any mutation whose Origin does not match the
/// Host makes the app's cross-origin posture explicit instead of relying solely
/// on the absence of CORS response headers. Requests without an Origin header,
/// such as curl, same-origin servers, and old clients, pass through because they
/// cannot carry ambient cross-site credentials.
pub async fn reject_cross_origin_mutations(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let origin_matches = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map_or(true, |origin| {
                let origin_host = origin
                    .strip_prefix("http://")
                    .or_else(|| origin.strip_prefix("https://"))
                    .unwrap_or(origin);
                origin_host.eq_ignore_ascii_case(host)
            });
        if !origin_matches {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"error\":\"cross-origin request rejected\"}"))
                .expect("cross origin response");
        }
    }
    next.run(request).await
}

pub async fn apply_rate_limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    let client_ip = request_client_ip(&request);
    let class_key = request_class_key(&request, &path);
    // The strictest budget guards the explicit auth endpoints; the moderate
    // budget guards every request that carries class credentials, each of which
    // triggers an Argon2 verification; everything else uses the global budget.
    // Keys combine the class id with the source address so rotating addresses
    // cannot cheaply reset a class-bound bucket.
    let allowed = if is_authentication_request(&method, &path) {
        // Two independent budgets are charged. The per-class bucket keeps one
        // noisy class from spending another's allowance, while the per-address
        // bucket is the one that actually holds: the class id is taken from the
        // URL, so keying only on class id plus address would hand an attacker a
        // fresh allowance for every id they invent. Both are evaluated so a
        // rejected request still consumes budget from each.
        let per_address_allowed = state
            .global_auth_rate_limiter
            .allow(&format!("auth-ip:{client_ip}"));
        let key = if class_key.is_empty() {
            format!("auth:create:{client_ip}")
        } else {
            format!("auth:{class_key}:{client_ip}")
        };
        let per_class_allowed = state.auth_rate_limiter.allow(&key);
        per_address_allowed && per_class_allowed
    } else if is_audit_chain_request(&method, &path) {
        state
            .audit_chain_rate_limiter
            .allow(&format!("chain:{client_ip}"))
    } else if carries_class_credentials(&request) {
        let key = if class_key.is_empty() {
            format!("class:{client_ip}")
        } else {
            format!("class:{class_key}:{client_ip}")
        };
        state.class_auth_rate_limiter.allow(&key)
    } else {
        state.rate_limiter.allow(&client_ip.to_string())
    };
    if !allowed {
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"error\":\"too many requests\"}"))
            .expect("rate limit response");
    }
    next.run(request).await
}

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; \
    connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

// Disable browser features the app never uses so embedded pages cannot be
// abused as a pivot for camera, microphone, location, or notification prompts.
const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), \
    payment=(), usb=(), battery=(), midi=(), sync-xhr=(), fullscreen=()";

/// Adds defensive response headers to every reply, including error replies.
/// API responses, meaning paths starting with /api/, also get Cache-Control:
/// no-store so shared proxies never cache sensitive data. Static assets keep
/// their own Cache-Control set by the static handler.
pub async fn add_security_headers(request: Request, next: Next) -> Response {
    let is_api_path = request.uri().path().starts_with("/api/") || request.uri().path() == "/api";
    // Metrics reveal request volumes and are not for shared caches either.
    let is_sensitive_path = is_api_path || request.uri().path() == "/metrics";
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    // Close the connection after every response. Without keep-alive, hyper tears
    // down the connection and its write buffers after each reply, so response
    // bodies, which may embed session tokens, do not linger in pooled memory
    // where a same-user local process could read them.
    headers.insert(
        HeaderName::from_static("connection"),
        HeaderValue::from_static("close"),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("0"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    // Cross-origin isolation headers prevent the document from being used in
    // cross-origin popups or embedded in frames on other origins.
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    // Prevent shared proxies from caching API or metrics responses that may
    // contain class names, operator names, financial data, or request volumes.
    if is_sensitive_path {
        headers.insert(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("no-store"),
        );
    }
    response
}
