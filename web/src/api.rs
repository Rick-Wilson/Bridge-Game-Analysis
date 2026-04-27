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

    // Run the BWS+PBN adapter, then the builder. BWS+PBN always produces one
    // session; we still keep the full list so the response shape matches
    // multi-session uploads from the JSON ingest endpoint.
    let normalized = bridge_club_analysis::data::adapters::pbn_bws::load_normalized(
        &bws_path,
        pbn_path.as_deref(),
        None,
    )
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Failed to parse files: {}", e),
        )
    })?;
    let session_list = bridge_club_analysis::build_sessions(&normalized, None).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Analysis error: {}", e),
        )
    })?;
    let game_data = session_list.first().map(|s| s.data.clone()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "No sessions in upload".to_string(),
    ))?;
    let session_infos: Vec<SessionInfo> = session_list
        .iter()
        .map(|s| SessionInfo {
            session_idx: s.session_idx,
            label: s.label.clone(),
            board_count: s.data.boards.len(),
            result_count: s.data.results.len(),
        })
        .collect();

    // Extract player list (keep placeholder names so player grid isn't empty)
    let mut players: Vec<String> = game_data
        .players
        .all_players()
        .map(|p| p.display_name())
        .collect();
    players.sort();
    players.dedup();

    // Build maps of display name -> ACBL number and placeholder list
    let mut player_acbl: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut missing_players: Vec<MissingPlayerInfo> = Vec::new();
    for p in game_data.players.all_players() {
        let display_name = p.display_name();
        let acbl = p.id.acbl_number.clone();
        if display_name.starts_with("Player ") {
            missing_players.push(MissingPlayerInfo {
                display_name,
                acbl_number: acbl,
            });
        } else if let Some(acbl) = acbl {
            player_acbl.insert(display_name, acbl);
        }
    }
    missing_players.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    missing_players.dedup_by(|a, b| a.display_name == b.display_name);

    // Count placeholder names (indicates the BWS had no ACBL name lookup)
    let missing_names = missing_players.len();

    // Build pair_num -> [acbl1, acbl2] map for the paste parser.
    // Key is the pair number as a string (for JSON compatibility).
    let mut pair_acbl: std::collections::HashMap<String, Vec<Option<String>>> =
        std::collections::HashMap::new();
    for ((_section, pair_num), (first_id, second_id)) in &game_data.pairs_by_number {
        if *pair_num <= 0 {
            continue;
        }
        pair_acbl.insert(
            pair_num.to_string(),
            vec![first_id.acbl_number.clone(), second_id.acbl_number.clone()],
        );
    }

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

    // Parse event date to a cleaner format if possible
    let event_date = game_data.event_date.as_ref().map(|d| {
        // BWS dates often look like "03/30/26 00:00:00" — extract just the date part
        let date_part = d.split(' ').next().unwrap_or(d);
        // Try to parse MM/DD/YY and reformat
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
    });

    Ok(Json(UploadResponse {
        session_id,
        event_name: game_data.event_name,
        event_date,
        players,
        board_count: boards.len(),
        boards,
        result_count,
        has_pbn: pbn_path.is_some(),
        missing_names,
        player_acbl,
        missing_players,
        pair_acbl,
        sessions: session_infos,
    }))
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
    let normalized = bridge_club_analysis::parse_normalized(body_str).map_err(|e| {
        let code = match e {
            bridge_club_analysis::SchemaParseError::UnsupportedMajor { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            _ => StatusCode::BAD_REQUEST,
        };
        (code, e.to_string())
    })?;

    // Adapter-emitted handviewer_urls (e.g. ACBL Live's "play this hand"
    // link, lifted by the extension) often render only the deal — no
    // auction. Override every result's URL with one built from the
    // schema's deal + contract + declarer + players via the same Rust
    // function the BWS adapter uses, so the BBO viewer shows our
    // constructed auction (passes-to-declarer, contract bid, closing
    // passes / X / XX).
    let mut normalized = normalized;
    bridge_club_analysis::enrich_handviewer_urls(&mut normalized);
    let body_str = serde_json::to_string(&normalized).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to re-serialize after enrichment: {}", e),
        )
    })?;

    let session_list = bridge_club_analysis::build_sessions(&normalized, None).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Failed to build sessions: {}", e),
        )
    })?;
    if session_list.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "No sessions found in normalized document".to_string(),
        ));
    }

    // Persist the document. Use a fresh UUID — JSON pushes don't append to
    // an existing session.
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_dir = state.upload_dir.join(&session_id);
    std::fs::create_dir_all(&session_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create session dir: {}", e),
        )
    })?;
    std::fs::write(session_dir.join("data.json"), &body_str).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to persist data.json: {}", e),
        )
    })?;

    // Build response identical to /api/upload (so frontend code branches once
    // on entry, not on input source).
    let game_data = session_list[0].data.clone();
    let session_infos: Vec<SessionInfo> = session_list
        .iter()
        .map(|s| SessionInfo {
            session_idx: s.session_idx,
            label: s.label.clone(),
            board_count: s.data.boards.len(),
            result_count: s.data.results.len(),
        })
        .collect();

    let mut players: Vec<String> = game_data
        .players
        .all_players()
        .map(|p| p.display_name())
        .collect();
    players.sort();
    players.dedup();

    let mut player_acbl: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut missing_players: Vec<MissingPlayerInfo> = Vec::new();
    for p in game_data.players.all_players() {
        let display_name = p.display_name();
        let acbl = p.id.acbl_number.clone();
        if display_name.starts_with("Player ") {
            missing_players.push(MissingPlayerInfo {
                display_name,
                acbl_number: acbl,
            });
        } else if let Some(acbl) = acbl {
            player_acbl.insert(display_name, acbl);
        }
    }
    missing_players.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    missing_players.dedup_by(|a, b| a.display_name == b.display_name);
    let missing_names = missing_players.len();

    let mut pair_acbl: std::collections::HashMap<String, Vec<Option<String>>> =
        std::collections::HashMap::new();
    for ((_section, pair_num), (first_id, second_id)) in &game_data.pairs_by_number {
        if *pair_num <= 0 {
            continue;
        }
        pair_acbl.insert(
            pair_num.to_string(),
            vec![first_id.acbl_number.clone(), second_id.acbl_number.clone()],
        );
    }

    let mut boards: Vec<u32> = game_data.results.iter().map(|r| r.board_number).collect();
    boards.sort();
    boards.dedup();
    let result_count = game_data.results.len();

    let duration = start.elapsed().as_millis() as u64;
    let logger = AuditLogger::new(&state.log_dir);
    logger.log_request(
        &anon_ip,
        "upload-normalized",
        &format!(
            "sessions={} boards={} results={}",
            session_list.len(),
            boards.len(),
            result_count
        ),
        &browser,
        &device,
        duration,
    );

    Ok(Json(UploadResponse {
        session_id,
        event_name: game_data.event_name,
        event_date: game_data.event_date,
        players,
        board_count: boards.len(),
        boards,
        result_count,
        has_pbn: true, // Normalized documents always carry full board data
        missing_names,
        player_acbl,
        missing_players,
        pair_acbl,
        sessions: session_infos,
    }))
}

/// List all sessions in an upload (BWS+PBN: always 1; JSON ingest: many).
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<SessionInfo>>, (StatusCode, String)> {
    let sessions = load_sessions(&state, &params)?;
    let infos = sessions
        .into_iter()
        .map(|s| SessionInfo {
            session_idx: s.session_idx,
            label: s.label,
            board_count: s.data.boards.len(),
            result_count: s.data.results.len(),
        })
        .collect();
    Ok(Json(infos))
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

    let mut response: BoardAnalysisResponse = (&analysis).into();

    // Generate per-row BBO URLs and board-level deal info
    if let Some(board_data) = game_data.boards.get(&board_num) {
        // Per-row BBO URLs: match response rows to raw results by NS pair names
        for resp_row in &mut response.results {
            let raw_match = game_data.results.iter().find(|r| {
                r.board_number == board_num
                    && r.ns_pair.first_player().display_name() == resp_row.ns_player1
                    && r.ns_pair.second_player().display_name() == resp_row.ns_player2
            });
            if let Some(result) = raw_match {
                let seat_players = SeatPlayers::from_partnerships(&result.ns_pair, &result.ew_pair);
                let contract_result = result.contract.as_ref().map(|c| ContractResult {
                    contract: c.clone(),
                    declarer: result.declarer_direction,
                });
                resp_row.bbo_url =
                    board_data.bbo_handviewer_url(Some(&seat_players), contract_result.as_ref());
            }
        }

        // Use first result's URL as the board-level default
        response.bbo_url = response.results.first().and_then(|r| r.bbo_url.clone());

        // Deal info for DD table and BBA auction
        if let Some(deal) = &board_data.deal {
            let vul_str = match board_data.vulnerability {
                bridge_club_analysis::Vulnerability::None => "None",
                bridge_club_analysis::Vulnerability::NorthSouth => "NS",
                bridge_club_analysis::Vulnerability::EastWest => "EW",
                bridge_club_analysis::Vulnerability::Both => "Both",
            };
            let dealer_str = match board_data.dealer {
                bridge_club_analysis::Direction::North => "N",
                bridge_club_analysis::Direction::South => "S",
                bridge_club_analysis::Direction::East => "E",
                bridge_club_analysis::Direction::West => "W",
            };
            let (par_contract, optimum_score) =
                bridge_club_analysis::render_par_display(&board_data.par);
            response.deal_info = Some(BoardDealInfo {
                pbn: Some(deal.to_pbn(board_data.dealer)),
                dealer: dealer_str.to_string(),
                vulnerability: vul_str.to_string(),
                par_contract,
                optimum_score,
            });
        }
    }

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

/// Update player name overrides for a session.
///
/// Body: JSON map of ACBL number -> name, e.g. {"2176661": "David Bailey", ...}.
/// Merged into the session's names.json file, which is applied on every
/// subsequent API request.
pub async fn update_names(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    Json(new_names): Json<HashMap<String, String>>,
) -> Result<Json<UpdateNamesResponse>, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;

    // Validate session ID (UUID format)
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

    // Load existing names.json if present, then merge in the new names
    let names_path = session_dir.join("names.json");
    let mut existing: HashMap<String, String> = if names_path.exists() {
        std::fs::read_to_string(&names_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    for (acbl, name) in new_names {
        if !acbl.trim().is_empty() && !name.trim().is_empty() {
            existing.insert(acbl.trim().to_string(), name.trim().to_string());
        }
    }

    let json = serde_json::to_string_pretty(&existing).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize names: {}", e),
        )
    })?;
    std::fs::write(&names_path, json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write names.json: {}", e),
        )
    })?;

    Ok(Json(UpdateNamesResponse {
        total_names: existing.len(),
    }))
}

/// Return the full normalized JSON document for a session as raw bytes.
///
/// For data.json sessions (extension JSON pushes), streams the file directly.
/// For BWS+PBN sessions, runs the adapter on demand and serializes the result.
/// Used by the JS port's parity-test harness — the SPA fetches this once
/// after upload, then runs client-side analysis against it and compares to
/// the server's per-player / per-board responses.
pub async fn get_normalized(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let session_id = params.get("session").ok_or((
        StatusCode::BAD_REQUEST,
        "Missing 'session' parameter".to_string(),
    ))?;
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

    // Prefer the original JSON when it was uploaded directly (extension path) —
    // it preserves any source-side fields the analyzer doesn't model.
    let data_json_path = session_dir.join("data.json");
    if data_json_path.exists() {
        let body = std::fs::read_to_string(&data_json_path).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read data.json: {}", e),
            )
        })?;
        return Ok((
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        ));
    }

    // BWS+PBN path: run the adapter, serialize.
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
    let names_path = session_dir.join("names.json");
    let name_overrides: Option<HashMap<String, String>> = if names_path.exists() {
        std::fs::read_to_string(&names_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };
    let normalized = bridge_club_analysis::data::adapters::pbn_bws::load_normalized(
        &bws_path,
        pbn_path.as_deref(),
        name_overrides.as_ref(),
    )
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Failed to parse BWS/PBN: {}", e),
        )
    })?;
    let body = serde_json::to_string(&normalized).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize: {}", e),
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

/// Load all sessions for an upload UUID. BWS+PBN uploads always produce
/// one session; the JSON ingest endpoint can produce many.
fn load_sessions(
    state: &AppState,
    params: &HashMap<String, String>,
) -> Result<Vec<bridge_club_analysis::SessionData>, (StatusCode, String)> {
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

    // Load name overrides if names.json exists in the session directory.
    let names_path = session_dir.join("names.json");
    let name_overrides: Option<HashMap<String, String>> = if names_path.exists() {
        std::fs::read_to_string(&names_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    // Prefer normalized JSON if present (extension uploads + future paths);
    // fall back to BWS+PBN scan (legacy upload flow).
    let data_json_path = session_dir.join("data.json");
    if data_json_path.exists() {
        let body = std::fs::read_to_string(&data_json_path).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read data.json: {}", e),
            )
        })?;
        let game = bridge_club_analysis::parse_normalized(&body).map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Invalid normalized JSON: {}", e),
            )
        })?;
        return bridge_club_analysis::build_sessions(&game, name_overrides.as_ref()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Analysis error: {}", e),
            )
        });
    }

    // Find BWS and PBN files in session directory.
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

    // Run the BWS+PBN adapter, then the builder; this also produces a
    // SessionData list (single entry for BWS).
    let normalized = bridge_club_analysis::data::adapters::pbn_bws::load_normalized(
        &bws_path,
        pbn_path.as_deref(),
        name_overrides.as_ref(),
    )
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Failed to parse BWS/PBN: {}", e),
        )
    })?;
    bridge_club_analysis::build_sessions(&normalized, None).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Analysis error: {}", e),
        )
    })
}

/// Resolve which session the request targets (`session_idx` query param,
/// default 0) and return owned SessionData. Errors if the index is out of range.
fn pick_session(
    mut sessions: Vec<bridge_club_analysis::SessionData>,
    params: &HashMap<String, String>,
) -> Result<bridge_club_analysis::SessionData, (StatusCode, String)> {
    if sessions.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No sessions in upload".to_string()));
    }
    let idx: usize = params
        .get("session_idx")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if idx >= sessions.len() {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Session index {} out of range (have {})",
                idx,
                sessions.len()
            ),
        ));
    }
    Ok(sessions.swap_remove(idx))
}

/// Convenience: load sessions and pick the targeted one in one call.
fn load_session_data(
    state: &AppState,
    params: &HashMap<String, String>,
) -> Result<bridge_club_analysis::GameData, (StatusCode, String)> {
    let sessions = load_sessions(state, params)?;
    let session = pick_session(sessions, params)?;
    Ok(session.data)
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
