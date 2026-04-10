//! API route handlers for the bridge analysis web server.

use crate::analytics::{self, AuditLogger};
use crate::responses::*;
use crate::AppState;
use axum::{
    extract::{ConnectInfo, Multipart, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json},
};
use bridge_club_analysis::{ContractResult, SeatPlayers};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Serve the ACBL download help screenshot.
pub async fn acbl_help_image() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/png")],
        include_bytes!("../static/acbl-download-help.png").as_slice(),
    )
}

/// Health check endpoint.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// Serve the main SPA page.
pub async fn index_page(State(state): State<Arc<AppState>>) -> Html<String> {
    // Try disk first (hot-reload in dev), fall back to embedded
    let disk_path = Path::new("web/static/index.html");
    let html = if disk_path.exists() {
        std::fs::read_to_string(disk_path)
            .unwrap_or_else(|_| include_str!("../static/index.html").to_string())
    } else {
        include_str!("../static/index.html").to_string()
    };
    // Inject base path for API calls
    let html = html.replace("{{BASE_PATH}}", &state.base_path);
    Html(html)
}

/// Upload BWS and optional PBN files. Returns session ID + player/board lists.
/// If `add_to` query param is set, adds files to an existing session instead.
pub async fn upload_files(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let raw_ip = analytics::extract_ip(&headers, &addr);
    let anon_ip = analytics::anonymize_ip(&raw_ip);
    let (browser, device) = analytics::extract_user_agent_info(&headers);

    // Use existing session or create new one
    let session_id = if let Some(existing) = params.get("add_to") {
        existing.clone()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let session_dir = state.upload_dir.join(&session_id);
    std::fs::create_dir_all(&session_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create session dir: {}", e),
        )
    })?;

    // Save uploaded files to session directory
    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or("upload").to_string();
        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to read field: {}", e),
            )
        })?;

        let dest = session_dir.join(&file_name);
        std::fs::write(&dest, &data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write file: {}", e),
            )
        })?;
    }

    // Find BWS and PBN files in session directory
    let mut bws_path = None;
    let mut pbn_path = None;
    if let Ok(entries) = std::fs::read_dir(&session_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            match ext.as_str() {
                "bws" => bws_path = Some(path),
                "pbn" => pbn_path = Some(path),
                _ => {}
            }
        }
    }

    let bws_path = bws_path.ok_or((StatusCode::BAD_REQUEST, "No BWS file uploaded".to_string()))?;

    // Load and analyze
    let game_data = bridge_club_analysis::load_game_data(&bws_path, pbn_path.as_deref(), None)
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Failed to parse files: {}", e),
            )
        })?;

    // Extract player list
    let mut players: Vec<String> = game_data
        .players
        .all_players()
        .map(|p| p.display_name())
        .filter(|name| !name.starts_with("Player ")) // Filter placeholders
        .collect();
    players.sort();
    players.dedup();

    // Extract board list
    let mut boards: Vec<u32> = game_data.results.iter().map(|r| r.board_number).collect();
    boards.sort();
    boards.dedup();

    let result_count = game_data.results.len();

    // Save game data as serialized file for later API calls
    // (We re-parse on each request for now — simple approach)

    let duration = start.elapsed().as_millis() as u64;
    let logger = AuditLogger::new(&state.log_dir);
    logger.log_request(
        &anon_ip,
        "upload",
        &format!("boards={} results={}", boards.len(), result_count),
        &browser,
        &device,
        duration,
    );

    Ok(Json(UploadResponse {
        session_id,
        players,
        board_count: boards.len(),
        boards,
        result_count,
    }))
}

/// List players for a session.
pub async fn list_players(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<PlayerListResponse>, (StatusCode, String)> {
    let game_data = load_session_data(&state, &params)?;
    let mut players: Vec<String> = game_data
        .players
        .all_players()
        .map(|p| p.display_name())
        .filter(|name| !name.starts_with("Player "))
        .collect();
    players.sort();
    players.dedup();
    Ok(Json(PlayerListResponse { players }))
}

/// List boards for a session.
pub async fn list_boards(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<BoardListResponse>, (StatusCode, String)> {
    let game_data = load_session_data(&state, &params)?;
    let mut boards: Vec<u32> = game_data.results.iter().map(|r| r.board_number).collect();
    boards.sort();
    boards.dedup();
    Ok(Json(BoardListResponse { boards }))
}

/// Analyze a specific player.
pub async fn analyze_player(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<PlayerAnalysisResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let raw_ip = analytics::extract_ip(&headers, &addr);
    let anon_ip = analytics::anonymize_ip(&raw_ip);
    let (browser, device) = analytics::extract_user_agent_info(&headers);

    let name = params.get("name").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'name' parameter".to_string(),
    ))?;

    let game_data = load_session_data(&state, &params)?;
    let analysis = bridge_club_analysis::analyze_player(&game_data, name).ok_or((
        StatusCode::NOT_FOUND,
        format!("Player '{}' not found", name),
    ))?;

    let mut response: PlayerAnalysisResponse = (&analysis).into();

    // Populate BBO hand viewer URLs
    for (i, br) in analysis.board_results.iter().enumerate() {
        if let Some(board_data) = game_data.boards.get(&br.board_number) {
            let board_result = game_data.results.iter().find(|r| {
                r.board_number == br.board_number
                    && (r.ns_pair.contains(&analysis.player)
                        || r.ew_pair.contains(&analysis.player))
            });

            let seat_players =
                board_result.map(|r| SeatPlayers::from_partnerships(&r.ns_pair, &r.ew_pair));
            let contract_result = board_result.and_then(|r| {
                r.contract.as_ref().map(|c| ContractResult {
                    contract: c.clone(),
                    declarer: r.declarer_direction,
                })
            });

            if let Some(url) =
                board_data.bbo_handviewer_url(seat_players.as_ref(), contract_result.as_ref())
            {
                response.board_results[i].bbo_url = Some(url);
            }
        }
    }

    let duration = start.elapsed().as_millis() as u64;
    let logger = AuditLogger::new(&state.log_dir);
    logger.log_request(&anon_ip, "player", name, &browser, &device, duration);

    Ok(Json(response))
}

/// Analyze a specific board.
pub async fn analyze_board(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<BoardAnalysisResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let raw_ip = analytics::extract_ip(&headers, &addr);
    let anon_ip = analytics::anonymize_ip(&raw_ip);
    let (browser, device) = analytics::extract_user_agent_info(&headers);

    let board_num: u32 = params
        .get("num")
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing 'num' parameter".to_string(),
        ))?
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid board number".to_string()))?;

    let game_data = load_session_data(&state, &params)?;
    let analysis = bridge_club_analysis::analyze_board(&game_data, board_num).ok_or((
        StatusCode::NOT_FOUND,
        format!("Board {} not found", board_num),
    ))?;

    let response: BoardAnalysisResponse = (&analysis).into();

    let duration = start.elapsed().as_millis() as u64;
    let logger = AuditLogger::new(&state.log_dir);
    logger.log_request(
        &anon_ip,
        "board",
        &board_num.to_string(),
        &browser,
        &device,
        duration,
    );

    Ok(Json(response))
}

// ==================== Admin ====================

/// Admin dashboard page.
pub async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, StatusCode> {
    check_admin_access(&state, &headers, &addr, &params)?;

    let disk_path = Path::new("web/static/dashboard.html");
    let html = if disk_path.exists() {
        std::fs::read_to_string(disk_path)
            .unwrap_or_else(|_| include_str!("../static/dashboard.html").to_string())
    } else {
        include_str!("../static/dashboard.html").to_string()
    };
    let html = html.replace("{{BASE_PATH}}", &state.base_path);
    Ok(Html(html))
}

/// Admin stats API.
pub async fn admin_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, StatusCode> {
    check_admin_access(&state, &headers, &addr, &params)?;
    let logger = AuditLogger::new(&state.log_dir);
    Ok(Json(logger.get_stats()))
}

/// Admin log file list.
pub async fn admin_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, StatusCode> {
    check_admin_access(&state, &headers, &addr, &params)?;
    let logger = AuditLogger::new(&state.log_dir);
    Ok(Json(logger.list_logs()))
}

// ==================== Helpers ====================

/// Load game data from a session's uploaded files.
fn load_session_data(
    state: &AppState,
    params: &HashMap<String, String>,
) -> Result<bridge_club_analysis::GameData, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;

    // Validate session ID format (UUID)
    if session_id.len() != 36
        || !session_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid session ID".to_string()));
    }

    let session_dir = state.upload_dir.join(session_id);
    if !session_dir.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            "Session not found or expired".to_string(),
        ));
    }

    // Find BWS and PBN files in session directory
    let mut bws_path = None;
    let mut pbn_path = None;
    if let Ok(entries) = std::fs::read_dir(&session_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            match ext.as_str() {
                "bws" => bws_path = Some(path),
                "pbn" => pbn_path = Some(path),
                _ => {}
            }
        }
    }

    let bws_path = bws_path.ok_or((
        StatusCode::NOT_FOUND,
        "BWS file not found in session".to_string(),
    ))?;

    bridge_club_analysis::load_game_data(&bws_path, pbn_path.as_deref(), None).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Analysis error: {}", e),
        )
    })
}

/// Check admin access via admin key or localhost.
fn check_admin_access(
    state: &AppState,
    headers: &HeaderMap,
    addr: &SocketAddr,
    params: &HashMap<String, String>,
) -> Result<(), StatusCode> {
    let ip = analytics::extract_ip(headers, addr);

    // Allow localhost
    if ip == "127.0.0.1" || ip == "::1" {
        return Ok(());
    }

    // Check admin key
    if let Some(ref admin_key) = state.admin_key {
        if let Some(key) = params.get("key") {
            if key == admin_key {
                return Ok(());
            }
        }
    }

    Err(StatusCode::FORBIDDEN)
}
