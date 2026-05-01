//! Prometheus HTTP metrics matching the bridge-craftwork service contract:
//! `http_requests_total{method,path,status}` counter and
//! `http_request_duration_seconds{method,path}` histogram. The `track`
//! middleware records both per request; `render` produces the Prometheus
//! text-format output for `/metrics`.

use std::time::Instant;

use axum::{body::Body, extract::MatchedPath, http::Request, middleware::Next, response::Response};
use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec, Registry,
    TextEncoder,
};

pub static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

pub static HTTP_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let v = register_int_counter_vec!(
        "http_requests_total",
        "Total HTTP requests",
        &["method", "path", "status"]
    )
    .expect("register http_requests_total");
    REGISTRY
        .register(Box::new(v.clone()))
        .expect("register counter in registry");
    v
});

pub static HTTP_REQUEST_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    let v = register_histogram_vec!(
        "http_request_duration_seconds",
        "HTTP request duration",
        &["method", "path"]
    )
    .expect("register http_request_duration_seconds");
    REGISTRY
        .register(Box::new(v.clone()))
        .expect("register histogram in registry");
    v
});

/// Force-init the lazy metrics so they show up in `/metrics` even before
/// the first request lands.
pub fn init() {
    Lazy::force(&HTTP_REQUESTS_TOTAL);
    Lazy::force(&HTTP_REQUEST_DURATION);
}

/// Render the Prometheus text-format payload for the `/metrics` route.
pub fn render() -> String {
    let mut buf = String::new();
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    encoder
        .encode_utf8(&metric_families, &mut buf)
        .expect("encode metrics");
    buf
}

/// Axum middleware. Wire up via `axum::middleware::from_fn(metrics::track)`.
pub async fn track(req: Request<Body>, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let started = Instant::now();
    let response = next.run(req).await;
    let elapsed = started.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    HTTP_REQUESTS_TOTAL
        .with_label_values(&[&method, &path, &status])
        .inc();
    HTTP_REQUEST_DURATION
        .with_label_values(&[&method, &path])
        .observe(elapsed);

    response
}
