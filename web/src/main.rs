mod analytics;
mod api;
mod responses;

use axum::{
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

/// Shared application state
pub struct AppState {
    /// Directory for temporary uploaded files
    pub upload_dir: PathBuf,
    /// Directory for audit/analytics logs
    pub log_dir: PathBuf,
    /// Admin key for dashboard access
    pub admin_key: Option<String>,
    /// Base path for serving (e.g., "/club-game-analysis")
    pub base_path: String,
}

#[tokio::main]
async fn main() {
    // Load .env if present
    let _ = dotenvy::dotenv();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);
    let base_path =
        std::env::var("BASE_PATH").unwrap_or_else(|_| "/club-game-analysis/app".to_string());
    let admin_key = std::env::var("ADMIN_KEY").ok();

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
        admin_key,
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

    // API routes
    let api_routes = Router::new()
        .route("/upload", post(api::upload_files))
        .route("/players", get(api::list_players))
        .route("/boards", get(api::list_boards))
        .route("/player", get(api::analyze_player))
        .route("/board", get(api::analyze_board))
        .route("/bba-proxy", post(api::bba_proxy));

    // Admin routes
    let admin_routes = Router::new()
        .route("/dashboard", get(api::admin_dashboard))
        .route("/api/stats", get(api::admin_stats))
        .route("/api/logs", get(api::admin_logs));

    // Main app with base path
    let app = Router::new()
        .route("/", get(api::index_page))
        .route("/static/acbl-download-help.png", get(api::acbl_help_image))
        .nest("/api", api_routes)
        .nest("/admin", admin_routes)
        .route("/health", get(api::health))
        .layer(cors)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024)) // 20MB for BWS files
        .with_state(state);

    // Nest under base path. Also handle trailing slash by redirecting.
    let redirect_to = base_path.clone();
    let root = Router::new().nest(&base_path, app).route(
        &format!("{}/", base_path),
        get(move || async move { axum::response::Redirect::permanent(&redirect_to) }),
    );

    let addr = SocketAddr::new(host.parse().expect("Invalid HOST"), port);
    tracing::info!("Starting server at http://{}{}", addr, base_path);

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
