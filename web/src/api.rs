//! API route handlers for the bridge analysis web server.

use crate::analytics::{self, AuditLogger};
use crate::responses::*;
use crate::upload_helpers;
use crate::AppState;
use axum::{
    extract::{ConnectInfo, Multipart, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json},
};
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

/// Service-contract health endpoint. Returns service status, version, and
/// uptime for ops tooling to scrape.
pub async fn healthz(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: state.version,
        uptime_seconds: state.started_at.elapsed().as_secs(),
    })
}

/// Prometheus text-format metrics endpoint.
pub async fn metrics() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        crate::observability::metrics::render(),
    )
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

    // Run the BWS+PBN adapter to produce a NormalizedGame, then enrich
    // (derive missing tricks from score; replace adapter-supplied
    // handviewer URLs with canonical BBO URLs that include a constructed
    // auction). After this, everything downstream works off the schema
    // alone — the JS analyzer in the SPA reads the same JSON.
    let mut normalized =
        parse_files::data::adapters::pbn_bws::load_normalized(&bws_path, pbn_path.as_deref(), None)
            .map_err(|e| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Failed to parse files: {}", e),
                )
            })?;
    parse_files::enrich_tricks(&mut normalized);
    parse_files::enrich_handviewer_urls(&mut normalized);
    persist_data_json(&session_dir, &normalized)?;

    let response = build_upload_response(
        session_id.clone(),
        &normalized,
        pbn_path.is_some(),
        &state,
        &anon_ip,
        &browser,
        &device,
        "upload",
        start,
    )?;
    Ok(Json(response))
}

/// Accept a normalized JSON document (schema 1.x) from the browser
/// extension, validate the schema version, and persist it as a session.
/// Response mirrors `upload_files` so the frontend can take the same path.
pub async fn upload_normalized(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let start = Instant::now();
    let raw_ip = analytics::extract_ip(&headers, &addr);
    let anon_ip = analytics::anonymize_ip(&raw_ip);
    let (browser, device) = analytics::extract_user_agent_info(&headers);

    let body_str = std::str::from_utf8(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "body is not valid UTF-8".to_string(),
        )
    })?;

    // Parse + version-check first so we never write garbage to disk.
    let mut normalized = parse_files::parse_normalized(body_str).map_err(|e| {
        let code = match e {
            parse_files::SchemaParseError::UnsupportedMajor { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            _ => StatusCode::BAD_REQUEST,
        };
        (code, e.to_string())
    })?;

    // Derive missing trick counts from score; replace adapter-supplied
    // handviewer URLs with canonical BBO URLs (constructed auction from
    // passes-to-declarer + contract + closing passes / X / XX).
    parse_files::enrich_tricks(&mut normalized);
    parse_files::enrich_handviewer_urls(&mut normalized);

    if upload_helpers::flatten_sessions(&normalized).is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "No sessions found in normalized document".to_string(),
        ));
    }

    // Use a fresh UUID — JSON pushes don't append to an existing session.
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_dir = state.upload_dir.join(&session_id);
    std::fs::create_dir_all(&session_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create session dir: {}", e),
        )
    })?;
    persist_data_json(&session_dir, &normalized)?;

    let response = build_upload_response(
        session_id,
        &normalized,
        true, // Normalized documents always carry full board data.
        &state,
        &anon_ip,
        &browser,
        &device,
        "upload-normalized",
        start,
    )?;
    Ok(Json(response))
}

/// List all sessions in an upload (BWS+PBN: always 1; JSON ingest: many).
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<SessionInfo>>, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;
    let session_dir = resolve_session_dir(&state, session_id)?;
    let game = read_data_json(&session_dir)?;
    let infos: Vec<SessionInfo> = upload_helpers::flatten_sessions(&game)
        .into_iter()
        .map(|s| SessionInfo {
            session_idx: s.session_idx,
            label: s.label,
            board_count: s.session.boards.len(),
            result_count: upload_helpers::result_count(s.session),
        })
        .collect();
    Ok(Json(infos))
}

/// Proxy BBA auction requests to avoid CORS issues.
pub async fn bba_proxy(body: axum::body::Bytes) -> Result<impl IntoResponse, (StatusCode, String)> {
    let client = reqwest::Client::new();
    let res = client
        .post("https://bba.harmonicsystems.com/api/auction/generate")
        .header("Content-Type", "application/json")
        .header("X-Client-Version", "ClubGameAnalysis")
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("BBA request failed: {}", e),
            )
        })?;

    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = res.bytes().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("BBA response error: {}", e),
        )
    })?;

    Ok((
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    ))
}

/// Apply player name overrides (acbl_number → display name) to a session.
///
/// Body: JSON map of ACBL number → name, e.g. {"2176661": "David Bailey"}.
/// Mutates `data.json` in place so subsequent /api/normalized reads see
/// the override; also re-runs the handviewer-URL enrichment so the
/// constructed BBO URL embeds the new names.
pub async fn update_names(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    Json(new_names): Json<HashMap<String, String>>,
) -> Result<Json<UpdateNamesResponse>, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;
    let session_dir = resolve_session_dir(&state, session_id)?;

    // Filter empty entries and trim whitespace.
    let overrides: HashMap<String, String> = new_names
        .into_iter()
        .filter_map(|(acbl, name)| {
            let acbl = acbl.trim().to_string();
            let name = name.trim().to_string();
            if acbl.is_empty() || name.is_empty() {
                None
            } else {
                Some((acbl, name))
            }
        })
        .collect();

    let mut game = read_data_json(&session_dir)?;
    let applied = upload_helpers::apply_name_overrides(&mut game, &overrides);
    if applied > 0 {
        // Names appear in the constructed BBO handviewer URLs; rebuild them.
        parse_files::enrich_handviewer_urls(&mut game);
    }
    persist_data_json(&session_dir, &game)?;

    Ok(Json(UpdateNamesResponse {
        total_names: applied,
    }))
}

/// Return the full normalized JSON document for a session as raw bytes.
///
/// Every session — whether it came from BWS+PBN multipart or from a
/// JSON push from the browser extension — is persisted as `data.json`
/// at upload time, so this endpoint just streams that file.
pub async fn get_normalized(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;
    let session_dir = resolve_session_dir(&state, session_id)?;
    let body = std::fs::read_to_string(session_dir.join("data.json")).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "Session has no data.json (uploaded before the schema-only refactor?)".to_string(),
        )
    })?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    ))
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

/// Validate the session-id query param and return the session directory.
fn resolve_session_dir(
    state: &AppState,
    session_id: &str,
) -> Result<std::path::PathBuf, (StatusCode, String)> {
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
    Ok(session_dir)
}

/// Read the persisted normalized JSON for a session.
fn read_data_json(
    session_dir: &std::path::Path,
) -> Result<parse_files::NormalizedGame, (StatusCode, String)> {
    let body = std::fs::read_to_string(session_dir.join("data.json")).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "Session has no data.json (uploaded before the schema-only refactor?)".to_string(),
        )
    })?;
    parse_files::parse_normalized(&body).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid normalized JSON in session: {}", e),
        )
    })
}

/// Serialize and persist the normalized JSON for a session.
fn persist_data_json(
    session_dir: &std::path::Path,
    game: &parse_files::NormalizedGame,
) -> Result<(), (StatusCode, String)> {
    let body = serde_json::to_string(game).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize data.json: {}", e),
        )
    })?;
    std::fs::write(session_dir.join("data.json"), body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write data.json: {}", e),
        )
    })
}

/// Build the upload response from a freshly-enriched NormalizedGame and
/// log an audit row. Used by both /api/upload and /api/upload-normalized
/// so the response shape is identical.
#[allow(clippy::too_many_arguments)]
fn build_upload_response(
    session_id: String,
    game: &parse_files::NormalizedGame,
    has_pbn: bool,
    state: &AppState,
    anon_ip: &str,
    browser: &str,
    device: &str,
    log_action: &str,
    start: Instant,
) -> Result<UploadResponse, (StatusCode, String)> {
    let flat = upload_helpers::flatten_sessions(game);
    let first = flat.first().ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        "No sessions in upload".to_string(),
    ))?;

    let session_infos: Vec<SessionInfo> = flat
        .iter()
        .map(|s| SessionInfo {
            session_idx: s.session_idx,
            label: s.label.clone(),
            board_count: s.session.boards.len(),
            result_count: upload_helpers::result_count(s.session),
        })
        .collect();

    let summary = upload_helpers::summarize_players(first.session);
    let boards = upload_helpers::board_numbers(first.session);
    let result_count = upload_helpers::result_count(first.session);

    let event_date = first.event_date.as_deref().map(reformat_event_date);

    let duration = start.elapsed().as_millis() as u64;
    let logger = AuditLogger::new(&state.log_dir);
    logger.log_request(
        anon_ip,
        log_action,
        &format!(
            "sessions={} boards={} results={}",
            flat.len(),
            boards.len(),
            result_count
        ),
        browser,
        device,
        duration,
    );

    let missing_names = summary.missing_players.len();
    Ok(UploadResponse {
        session_id,
        event_name: first.event_name.clone(),
        event_date,
        players: summary.display_names,
        board_count: boards.len(),
        boards,
        result_count,
        has_pbn,
        missing_names,
        player_acbl: summary.player_acbl,
        missing_players: summary.missing_players,
        pair_acbl: summary.pair_acbl,
        sessions: session_infos,
    })
}

/// BWS dates look like "03/30/26 00:00:00" — extract just the date part and
/// reformat as YYYY-MM-DD when possible. Pass-through otherwise.
fn reformat_event_date(d: &str) -> String {
    let date_part = d.split(' ').next().unwrap_or(d);
    let parts: Vec<&str> = date_part.split('/').collect();
    if parts.len() == 3 {
        let year = if parts[2].len() == 2 {
            format!("20{}", parts[2])
        } else {
            parts[2].to_string()
        };
        format!("{}-{}-{}", year, parts[0], parts[1])
    } else {
        date_part.to_string()
    }
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

    if let Some(ref dashboard_secret) = state.dashboard_secret {
        if let Some(key) = params.get("key") {
            if key == dashboard_secret {
                return Ok(());
            }
        }
    }

    Err(StatusCode::FORBIDDEN)
}
