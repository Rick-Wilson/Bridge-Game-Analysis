mod analytics;
mod api;
mod observability;
mod responses;

use axum::{
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method},
    middleware,
    response::Redirect,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;

/// Shared application state
pub struct AppState {
    /// Directory for temporary uploaded files
    pub upload_dir: PathBuf,
    /// Directory for audit/analytics logs
    pub log_dir: PathBuf,
    /// Secret for gating /admin/dashboard?key=… (matches the platform's
    /// DASHBOARD_SECRET env var)
    pub dashboard_secret: Option<String>,
    /// Base path for serving (e.g., "/club-game-analysis")
    pub base_path: String,
    /// Process start time, used by /healthz for uptime_seconds.
    pub started_at: Instant,
    /// Service version, used by /healthz.
    pub version: &'static str,
}

#[tokio::main]
async fn main() {
    // Load .env if present
    let _ = dotenvy::dotenv();

    // Service-contract logging: JSON to stdout in prod, pretty locally.
    let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());
    observability::logging::init(&log_level, &log_format);
    observability::metrics::init();

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);
    let base_path = std::env::var("BASE_PATH").unwrap_or_else(|_| String::new());
    let dashboard_secret = std::env::var("DASHBOARD_SECRET").ok();

    // Set up directories
    let upload_dir = std::env::var("UPLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("bridge-analysis-uploads"));
    let log_dir = std::env::var("LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("logs"));

    std::fs::create_dir_all(&upload_dir).expect("Failed to create upload directory");
    std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");

    let state = Arc::new(AppState {
        upload_dir,
        log_dir,
        base_path: base_path.clone(),
        dashboard_secret,
        started_at: Instant::now(),
        version: env!("CARGO_PKG_VERSION"),
    });

    // CORS - allow the domain and localhost for development
    let cors = CorsLayer::new()
        .allow_origin([
            "https://bridge-classroom.com"
                .parse::<HeaderValue>()
                .unwrap(),
            "http://localhost:3001".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(tower_http::cors::Any);

    // API routes. Analysis endpoints (/players, /boards, /player, /board)
    // were removed when the JS-side analyzer became the only path.
    let api_routes = Router::new()
        .route("/upload", post(api::upload_files))
        .route("/upload-normalized", post(api::upload_normalized))
        .route("/sessions", get(api::list_sessions))
        .route("/normalized", get(api::get_normalized))
        .route("/bba-proxy", post(api::bba_proxy))
        .route("/names", post(api::update_names));

    // Admin routes
    let admin_routes = Router::new()
        .route("/dashboard", get(api::admin_dashboard))
        .route("/api/stats", get(api::admin_stats))
        .route("/api/logs", get(api::admin_logs));

    // Main app with base path
    let app = Router::new()
        .route("/", get(api::index_page))
        .route("/analyze", get(api::index_page))
        .route("/static/acbl-download-help.png", get(api::acbl_help_image))
        .nest("/api", api_routes)
        .nest("/admin", admin_routes)
        .route("/healthz", get(api::healthz))
        .route("/metrics", get(api::metrics))
        // Transition alias — old monitors / scripts still hitting /health
        // get a 308 to /healthz. Drop once nothing external still uses it.
        .route("/health", get(|| async { Redirect::permanent("/healthz") }))
        .layer(middleware::from_fn(observability::metrics::track))
        .layer(cors)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024)) // 20MB for BWS files
        .with_state(state);

    // Nest under base path if set, otherwise serve at root
    let root = if base_path.is_empty() {
        app
    } else {
        let redirect_to = base_path.clone();
        Router::new().nest(&base_path, app).route(
            &format!("{}/", base_path),
            get(move || async move { axum::response::Redirect::permanent(&redirect_to) }),
        )
    };

    let addr = SocketAddr::new(host.parse().expect("Invalid HOST"), port);
    tracing::info!(
        "Starting server at http://{}{}",
        addr,
        if base_path.is_empty() {
            "/"
        } else {
            &base_path
        }
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");
    axum::serve(
        listener,
        root.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Server error");
}
