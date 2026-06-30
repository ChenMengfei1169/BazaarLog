// Prometheus metrics. Exposes a /metrics endpoint in text exposition format and
// counts inbound HTTP requests by method. Path labels are intentionally omitted
// to avoid cardinality explosions from ids in the URL.
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use once_cell::sync::Lazy;
use prometheus::{register_int_counter_vec, Encoder, IntCounterVec, TextEncoder};

pub static HTTP_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "bazaarlog_http_requests_total",
        "Total HTTP requests received by method.",
        &["method"]
    )
    .expect("register http_requests counter")
});

pub async fn metrics_handler() -> impl IntoResponse {
    // Touch the lazy so it is registered before gathering.
    Lazy::force(&HTTP_REQUESTS);
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    let _ = encoder.encode(&metric_families, &mut buffer);
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    )
}

// Lightweight middleware: increments the request counter and forwards. Kept
// allocation-free on the hot path.
pub async fn metrics_middleware(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    HTTP_REQUESTS.with_label_values(&[&method]).inc();
    next.run(req).await
}
