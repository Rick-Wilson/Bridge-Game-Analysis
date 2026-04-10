//! JSON response types for the web API.
//!
//! These convert from the analysis library's internal types (which don't
//! implement Serialize) to clean JSON-friendly structs.

use bridge_club_analysis::{
    BoardAnalysis, BoardTableResult, DirectionAnalysis, PlayerAnalysis, PlayerBoardResult,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub timestamp: String,
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
}

#[derive(Serialize)]
pub struct PlayerListResponse {
    pub players: Vec<String>,
}

#[derive(Serialize)]
pub struct BoardListResponse {
    pub boards: Vec<u32>,
}

// ==================== Player Analysis ====================

#[derive(Serialize)]
pub struct PlayerAnalysisResponse {
    pub player_name: String,
    pub partners: Vec<String>,
    pub seats: Vec<String>,
    pub boards_played: u32,
    pub boards_declared: u32,
    pub avg_matchpoint_pct: f64,
    pub declaring_mp_pct: Option<f64>,
    pub dummy_mp_pct: Option<f64>,
    pub defending_mp_pct: Option<f64>,
    pub avg_declarer_vs_field: Option<f64>,
    pub field_contract_pct: f64,
    pub board_results: Vec<PlayerBoardResultResponse>,
}

#[derive(Serialize)]
pub struct PlayerBoardResultResponse {
    pub board_number: u32,
    pub direction: String,
    pub seat: Option<String>,
    pub partner: String,
    pub contract: Option<String>,
    pub result_str: String,
    pub ns_score: i32,
    pub player_score: i32,
    pub matchpoint_pct: f64,
    pub role: String,
    pub declarer_vs_field: Option<f64>,
    pub field_contract: Option<String>,
    pub board_type: String,
    pub matched_field_contract: bool,
    pub cause: String,
    pub notes: String,
    pub bbo_url: Option<String>,
}

// ==================== Board Analysis ====================

#[derive(Serialize)]
pub struct BoardAnalysisResponse {
    pub board_number: u32,
    pub field_contract: Option<String>,
    pub board_type: String,
    pub results: Vec<BoardTableResultResponse>,
}

#[derive(Serialize)]
pub struct BoardTableResultResponse {
    pub ns_pair: String,
    pub ew_pair: String,
    pub contract: Option<String>,
    pub declarer_direction: String,
    pub result_str: String,
    pub ns_score: i32,
    pub ns_analysis: DirectionAnalysisResponse,
    pub ew_analysis: DirectionAnalysisResponse,
}

#[derive(Serialize)]
pub struct DirectionAnalysisResponse {
    pub matchpoint_pct: f64,
    pub role: String,
    pub declarer_vs_field: Option<f64>,
    pub matched_field_contract: bool,
    pub cause: String,
    pub notes: String,
}

// ==================== Conversions ====================

fn direction_str(d: bridge_club_analysis::PartnershipDirection) -> &'static str {
    match d {
        bridge_club_analysis::PartnershipDirection::NorthSouth => "NS",
        bridge_club_analysis::PartnershipDirection::EastWest => "EW",
    }
}

fn seat_str(d: bridge_club_analysis::Direction) -> &'static str {
    match d {
        bridge_club_analysis::Direction::North => "N",
        bridge_club_analysis::Direction::East => "E",
        bridge_club_analysis::Direction::South => "S",
        bridge_club_analysis::Direction::West => "W",
    }
}

fn role_str(r: bridge_club_analysis::PlayerRole) -> &'static str {
    match r {
        bridge_club_analysis::PlayerRole::Declarer => "Declarer",
        bridge_club_analysis::PlayerRole::Dummy => "Dummy",
        bridge_club_analysis::PlayerRole::Defender => "Defender",
    }
}

fn cause_str(c: bridge_club_analysis::ResultCause) -> &'static str {
    match c {
        bridge_club_analysis::ResultCause::Good => "Good",
        bridge_club_analysis::ResultCause::Lucky => "Lucky",
        bridge_club_analysis::ResultCause::Play => "Play",
        bridge_club_analysis::ResultCause::Defense => "Defense",
        bridge_club_analysis::ResultCause::Auction => "Auction",
        bridge_club_analysis::ResultCause::Unlucky => "Unlucky",
    }
}

impl From<&PlayerBoardResult> for PlayerBoardResultResponse {
    fn from(r: &PlayerBoardResult) -> Self {
        Self {
            board_number: r.board_number,
            direction: direction_str(r.direction).to_string(),
            seat: r.seat.map(|s| seat_str(s).to_string()),
            partner: r.partner.display_name(),
            contract: r.contract.as_ref().map(|c| c.display()),
            result_str: r.result_str.clone(),
            ns_score: r.ns_score,
            player_score: r.player_score,
            matchpoint_pct: r.matchpoint_pct,
            role: role_str(r.role).to_string(),
            declarer_vs_field: r.declarer_vs_field,
            field_contract: r.field_contract.as_ref().map(|c| c.display()),
            board_type: r.board_type.to_string(),
            matched_field_contract: r.matched_field_contract,
            cause: cause_str(r.cause).to_string(),
            notes: r.notes.clone(),
            bbo_url: None, // Populated by caller if board data available
        }
    }
}

impl From<&PlayerAnalysis> for PlayerAnalysisResponse {
    fn from(a: &PlayerAnalysis) -> Self {
        // Collect unique partners
        let mut partners: Vec<String> = a
            .board_results
            .iter()
            .map(|r| r.partner.display_name())
            .collect();
        partners.sort();
        partners.dedup();

        // Collect unique seats
        let mut seats: Vec<String> = a
            .board_results
            .iter()
            .filter_map(|r| r.seat.map(|s| seat_str(s).to_string()))
            .collect();
        seats.sort();
        seats.dedup();
        if seats.is_empty() {
            // Fallback to direction
            let mut dirs: Vec<String> = a
                .board_results
                .iter()
                .map(|r| direction_str(r.direction).to_string())
                .collect();
            dirs.sort();
            dirs.dedup();
            seats = dirs;
        }

        Self {
            player_name: a.player_name.clone(),
            partners,
            seats,
            boards_played: a.boards_played,
            boards_declared: a.boards_declared,
            avg_matchpoint_pct: a.avg_matchpoint_pct,
            declaring_mp_pct: a.declaring_mp_pct,
            dummy_mp_pct: a.dummy_mp_pct,
            defending_mp_pct: a.defending_mp_pct,
            avg_declarer_vs_field: a.avg_declarer_vs_field,
            field_contract_pct: a.field_contract_pct,
            board_results: a.board_results.iter().map(|r| r.into()).collect(),
        }
    }
}

impl From<&DirectionAnalysis> for DirectionAnalysisResponse {
    fn from(a: &DirectionAnalysis) -> Self {
        Self {
            matchpoint_pct: a.matchpoint_pct,
            role: role_str(a.role).to_string(),
            declarer_vs_field: a.declarer_vs_field,
            matched_field_contract: a.matched_field_contract,
            cause: cause_str(a.cause).to_string(),
            notes: a.notes.clone(),
        }
    }
}

impl From<&BoardTableResult> for BoardTableResultResponse {
    fn from(r: &BoardTableResult) -> Self {
        Self {
            ns_pair: r.ns_pair.display_name(),
            ew_pair: r.ew_pair.display_name(),
            contract: r.contract.as_ref().map(|c| c.display()),
            declarer_direction: seat_str(r.declarer_direction).to_string(),
            result_str: r.result_str.clone(),
            ns_score: r.ns_score,
            ns_analysis: (&r.ns_analysis).into(),
            ew_analysis: (&r.ew_analysis).into(),
        }
    }
}

impl From<&BoardAnalysis> for BoardAnalysisResponse {
    fn from(a: &BoardAnalysis) -> Self {
        Self {
            board_number: a.board_number,
            field_contract: a.field_contract.as_ref().map(|c| c.display()),
            board_type: a.board_type.to_string(),
            results: a.results.iter().map(|r| r.into()).collect(),
        }
    }
}
