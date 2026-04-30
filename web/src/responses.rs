//! JSON response types for the web API.

use serde::Serialize;

/// Service-contract /healthz response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub session_id: String,
    pub event_name: Option<String>,
    pub event_date: Option<String>,
    pub players: Vec<String>,
    pub boards: Vec<u32>,
    pub board_count: usize,
    pub result_count: usize,
    pub has_pbn: bool,
    /// Number of players with placeholder names (e.g., "Player N-1").
    /// Non-zero indicates the BWS file lacks ACBL name data.
    pub missing_names: usize,
    /// Map of display_name -> ACBL number for all players that have one.
    /// Used by the client to build up its localStorage dictionary.
    pub player_acbl: std::collections::HashMap<String, String>,
    /// Info about placeholder players, so the client can look up saved
    /// names in localStorage by ACBL number and auto-populate.
    pub missing_players: Vec<MissingPlayerInfo>,
    /// Pair-number lookup for the paste parser: pair_num -> [acbl1, acbl2]
    /// in display order (N-S or W-E). Only includes pairs where at least
    /// one seat has a placeholder name.
    pub pair_acbl: std::collections::HashMap<String, Vec<Option<String>>>,
    /// All sessions in the upload (always 1 for BWS+PBN, may be many for
    /// extension-pushed JSON). The browser uses session_idx to switch
    /// between them; analysis runs entirely client-side over the
    /// /api/normalized payload.
    pub sessions: Vec<SessionInfo>,
}

/// Session metadata for the session selector in the UI.
#[derive(Serialize)]
pub struct SessionInfo {
    pub session_idx: u32,
    pub label: String,
    pub board_count: usize,
    pub result_count: usize,
}

#[derive(Serialize)]
pub struct MissingPlayerInfo {
    pub display_name: String,
    pub acbl_number: Option<String>,
}

#[derive(Serialize)]
pub struct UpdateNamesResponse {
    pub total_names: usize,
}
