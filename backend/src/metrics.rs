// Prometheus metrics. Exposes a /metrics endpoint in text exposition format and
// counts inbound HTTP requests by method. Path labels are intentionally omitted
// to avoid cardinality explosions from ids in the URL.
//
// The endpoint is gated so internal metrics stay private: when
// BAZAARLOG_METRICS_TOKEN is set it requires a matching
// `Authorization: Bearer <token>` header; otherwise only loopback clients are
// allowed. This keeps /metrics unreadable on a LAN-deployed instance.
use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use once_cell::sync::Lazy;
use prometheus::{register_int_counter_vec, Encoder, IntCounterVec, TextEncoder};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub static HTTP_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "bazaarlog_http_requests_total",
        "Total HTTP requests received by method.",
        &["method"]
    )
    .expect("register http_requests counter")
});

pub async fn metrics_handler(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    request: Request,
) -> AppResult<impl IntoResponse> {
    if !is_metrics_allowed(&state, address.ip(), request.headers()) {
        return Err(AppError::Forbidden);
    }
    // Force the lazy so the counter is registered before gathering.
    Lazy::force(&HTTP_REQUESTS);
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|error| {
            AppError::Internal(anyhow::anyhow!("failed to encode metrics: {error}"))
        })?;
    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    ))
}

/// Constant-time byte comparison so a wrong but same-length token is not
/// distinguishable from a correct one through response timing.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left_byte, right_byte) in left.iter().zip(right.iter()) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

fn is_metrics_allowed(state: &AppState, client_ip: IpAddr, headers: &HeaderMap) -> bool {
    if let Some(token) = &state.config.metrics_token {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let expected = format!("Bearer {token}");
        return constant_time_eq(authorization.as_bytes(), expected.as_bytes());
    }
    client_ip.is_loopback()
}

/// Lightweight middleware: increments the request counter and forwards. Kept
/// allocation-free apart from the method label lookup.
pub async fn metrics_middleware(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    HTTP_REQUESTS.with_label_values(&[&method]).inc();
    next.run(request).await
}
