// Request hardening: defensive response headers and per-IP rate limiting.
// The global limiter blunts traffic abuse and DoS; the stricter limiter
// protects the password-checking endpoints from brute forcing.
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// Sliding-window rate limiter keyed by client IP address. The bucket map is
/// deliberately bounded so a flood of requests from many distinct source IPs
/// cannot grow memory without limit.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
    limit: usize,
    window: Duration,
    // Upper bound on tracked source IPs. Protects against memory exhaustion
    // when an attacker rotates many source addresses.
    max_entries: usize,
}

// Shared mutable state behind the limiter's mutex: the per-IP sliding-window
// queues plus the timestamp of the most recent bucket eviction.
struct RateLimiterInner {
    buckets: HashMap<IpAddr, VecDeque<Instant>>,
    last_eviction: Option<Instant>,
}

// Comfortably above any realistic single-host deployment while still bounding
// memory; each entry holds at most `limit` timestamps.
const MAX_TRACKED_CLIENTS: usize = 100_000;

impl RateLimiter {
    pub fn new(limit_times: usize, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                last_eviction: None,
            })),
            limit: limit_times,
            window: Duration::from_secs(window_secs),
            max_entries: MAX_TRACKED_CLIENTS,
        }
    }

    /// Returns true when `ip` still has budget inside the sliding window.
    pub fn allow(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let window_start = now - self.window;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let queue = inner.buckets.entry(ip).or_default();
        // Drop timestamps that have fallen out of the sliding window.
        while queue.front().is_some_and(|timestamp| *timestamp < window_start) {
            queue.pop_front();
        }
        if queue.len() >= self.limit {
            return false;
        }
        queue.push_back(now);
        // Bound memory. The eviction sweep runs at most once per window so a
        // flood of distinct source IPs cannot turn every request into an
        // O(n log n) pass while holding the lock.
        if inner.buckets.len() > self.max_entries
            && inner
                .last_eviction
                .map_or(true, |evicted_at| evicted_at < window_start)
        {
            inner.last_eviction = Some(now);
            inner
                .buckets
                .retain(|_, timestamps| timestamps.back().is_some_and(|latest| *latest >= window_start));
            // Still over the cap? Evict the least-recently-active buckets in a
            // single pass so the eviction cost stays O(n log n) per sweep.
            let excess = inner.buckets.len().saturating_sub(self.max_entries);
            if excess > 0 {
                // Collect owned keys and timestamps (both Copy) so the map can
                // be mutated while evicting, without holding borrows into it.
                let mut by_recency: Vec<(IpAddr, Instant)> = inner
                    .buckets
                    .iter()
                    .filter_map(|(ip, timestamps)| timestamps.back().map(|latest| (*ip, *latest)))
                    .collect();
                by_recency.sort_by_key(|(_, latest)| *latest);
                for (ip, _) in by_recency.into_iter().take(excess) {
                    inner.buckets.remove(&ip);
                }
            }
        }
        true
    }
}

// The password-checking endpoints deserve a stricter budget than the rest of
// the API. Only write requests are considered so the public class list stays
// on the global budget.
fn is_authentication_request(method: &Method, path: &str) -> bool {
    if method != Method::POST {
        return false;
    }
    path == "/api/classes" || (path.starts_with("/api/classes/") && path.ends_with("/auth"))
}

// Requests carrying class credentials run an Argon2 verification in the
// ClassAuth extractor. Matching on the class id header too is important: a
// request with x-class-id but no x-class-password still verifies the empty
// password against the stored hash, so it must consume the same tighter
// budget or an attacker could burn server CPU at twice the intended rate.
fn carries_class_credentials(request: &Request) -> bool {
    request.headers().contains_key("x-class-id")
        || request.headers().contains_key("x-class-password")
}

fn client_ip(request: &Request) -> IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip())
        .unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

// Explicit CSRF hardening: browsers attach an Origin header to
// state-changing cross-origin requests. Rejecting any mutation whose Origin
// does not match the Host makes the app's cross-origin posture explicit
// instead of relying solely on the absence of CORS response headers. Requests
// without an Origin header (curl, same-origin servers, old clients) pass
// through, as they cannot carry ambient cross-site credentials.
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
            .map(|origin| {
                let origin_host = origin
                    .strip_prefix("http://")
                    .or_else(|| origin.strip_prefix("https://"))
                    .unwrap_or(origin);
                origin_host.eq_ignore_ascii_case(host)
            })
            .unwrap_or(true);
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
    let ip = client_ip(&request);
    // The strictest budget guards the explicit auth endpoints; the moderate
    // budget guards every request that carries class credentials (each
    // triggers an Argon2 verification); everything else uses the global
    // budget.
    let allowed = if is_authentication_request(&method, &path) {
        state.auth_rate_limiter.allow(ip)
    } else if carries_class_credentials(&request) {
        state.class_auth_rate_limiter.allow(ip)
    } else {
        state.rate_limiter.allow(ip)
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
// abused as a pivot for camera/mic/location/notifications prompts.
const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), \
    payment=(), usb=(), battery=(), midi=(), sync-xhr=(), fullscreen=()";

/// Adds defensive response headers to every reply, including error replies.
/// API responses (paths starting with /api/) also get Cache-Control: no-store
/// so shared proxies never cache sensitive data. Static assets keep their own
/// Cache-Control set by the static handler.
pub async fn add_security_headers(request: Request, next: Next) -> Response {
    let is_api_path = request.uri().path().starts_with("/api/")
        || request.uri().path() == "/api";
    // Metrics reveal request volumes and are not for shared caches either.
    let is_sensitive_path = is_api_path || request.uri().path() == "/metrics";
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
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