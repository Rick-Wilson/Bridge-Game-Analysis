//! Serde types for the normalized JSON schema.
//!
//! Mirrors `docs/normalized-schema.md` (schema_version 1.0) from the
//! acbl-live-fetch project. This is the contract between data adapters
//! (PBN/BWS, ACBL Live extension, future sources) and the analyzer.
//!
//! All downstream analysis reads from these types; adapters' job ends at
//! producing a `NormalizedGame`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level container. One per JSON document, may carry many sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedGame {
    pub schema_version: String,
    pub source: String,
    pub fetched_at: String,
    /// URL of the page the data was scraped from; set by the extension, absent for file uploads.
    #[serde(default)]
    pub source_url: Option<String>,
    pub tournaments: Vec<Tournament>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tournament {
    #[serde(default)]
    pub sanction: Option<String>,
    #[serde(default)]
    pub schedule_url: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub scoring: Option<String>,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_number: u32,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub user_pair: Option<UserPair>,
    /// Optional pair-number → players map. Keys are stringified pair numbers.
    #[serde(default)]
    pub pairs: Option<HashMap<String, Vec<Player>>>,
    pub boards: Vec<Board>,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPair {
    #[serde(default)]
    pub section: Option<String>,
    pub direction: String,
    pub pair_number: i32,
    pub players: Vec<Player>,
    #[serde(default)]
    pub session_score: Option<f64>,
    #[serde(default)]
    pub session_percentage: Option<f64>,
    #[serde(default)]
    pub carryover: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub number: u32,
    #[serde(default)]
    pub section: Option<String>,
    pub dealer: String,
    pub vulnerability: String,
    #[serde(default)]
    pub deal: Option<Deal>,
    #[serde(default)]
    pub double_dummy: Option<DoubleDummy>,
    /// Par contracts. Schema 1.0 supports an array to represent ties
    /// (e.g., "N 4H= and S 4S= both score 420"). Empty when no par data.
    #[serde(default)]
    pub par: Vec<Par>,
    pub results: Vec<Result>,
    #[serde(default)]
    pub user_result_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deal {
    #[serde(rename = "N")]
    pub north: Hand,
    #[serde(rename = "E")]
    pub east: Hand,
    #[serde(rename = "S")]
    pub south: Hand,
    #[serde(rename = "W")]
    pub west: Hand,
}

/// Hand by suit. Each suit holds rank strings high-to-low; `[]` = void.
/// Ranks are uppercase, with `"10"` (not `"T"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hand {
    #[serde(rename = "S")]
    pub spades: Vec<String>,
    #[serde(rename = "H")]
    pub hearts: Vec<String>,
    #[serde(rename = "D")]
    pub diamonds: Vec<String>,
    #[serde(rename = "C")]
    pub clubs: Vec<String>,
}

/// Per-declarer double-dummy tricks. 4 declarers × 5 strains = 20 values.
/// Each value is the trick count (0–13) or null when unknown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubleDummy {
    #[serde(rename = "N", default)]
    pub north: Option<DdStrains>,
    #[serde(rename = "E", default)]
    pub east: Option<DdStrains>,
    #[serde(rename = "S", default)]
    pub south: Option<DdStrains>,
    #[serde(rename = "W", default)]
    pub west: Option<DdStrains>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdStrains {
    #[serde(rename = "C", default)]
    pub clubs: Option<u8>,
    #[serde(rename = "D", default)]
    pub diamonds: Option<u8>,
    #[serde(rename = "H", default)]
    pub hearts: Option<u8>,
    #[serde(rename = "S", default)]
    pub spades: Option<u8>,
    #[serde(rename = "NT", default)]
    pub no_trump: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Par {
    /// Signed integer; positive = NS gain.
    pub score: i32,
    /// Canonical contract string, e.g. "5NT", "4H", "6SX".
    pub contract: String,
    /// Best declarer for par ("N" | "E" | "S" | "W").
    pub declarer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Result {
    /// Canonical contract or "PASS"; null for no-result rows.
    #[serde(default)]
    pub contract: Option<String>,
    #[serde(default)]
    pub declarer: Option<String>,
    /// Tricks taken (0–13). Null only if score+contract can't determine it.
    #[serde(default)]
    pub tricks: Option<u8>,
    /// Signed integer; positive = NS gain. Null when no result was recorded.
    #[serde(default)]
    pub score: Option<i32>,
    #[serde(default)]
    pub matchpoints: Option<f64>,
    #[serde(default)]
    pub percentage: Option<f64>,
    #[serde(default)]
    pub imps: Option<f64>,
    pub ns_pair: Pair,
    pub ew_pair: Pair,
    #[serde(default)]
    pub auction: Option<Vec<String>>,
    #[serde(default)]
    pub play: Option<Vec<String>>,
    #[serde(default)]
    pub handviewer_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pair {
    #[serde(default)]
    pub number: Option<i32>,
    #[serde(default)]
    pub section: Option<String>,
    pub players: Vec<Player>,
    /// Stratification tier (1 = A, 2 = B, 3 = C …). Null when not available.
    #[serde(default)]
    pub strat: Option<i32>,
    /// Placements within each strat tier. Empty when not available.
    #[serde(default)]
    pub strat_ranks: Vec<StratRank>,
}

/// A strat-placement entry for a pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StratRank {
    pub strat: i32,
    pub rank: i32,
    pub scope: String,
}

/// Individual masterpoint award entry (one per pigment color).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterpointAward {
    pub amount: f64,
    /// ACBL pigment name: "Black", "Silver", "Red", "Gold", "Platinum", etc.
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    #[serde(default)]
    pub acbl_id: Option<String>,
    #[serde(default)]
    pub external_ids: HashMap<String, String>,
    /// Masterpoints awarded to this player for this session, broken down by color.
    /// Empty array when no award data is available.
    #[serde(default)]
    pub masterpoints_earned: Vec<MasterpointAward>,
}

/// The current major version this analyzer accepts.
pub const SUPPORTED_MAJOR: u32 = 1;

/// Parse a normalized JSON document and validate the schema version.
///
/// Accepts any minor version under the supported major (e.g. 1.0, 1.1, 1.2);
/// rejects unknown major versions.
pub fn parse_normalized(json: &str) -> std::result::Result<NormalizedGame, ParseError> {
    let game: NormalizedGame = serde_json::from_str(json).map_err(ParseError::Json)?;
    let major = game
        .schema_version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| ParseError::BadVersion(game.schema_version.clone()))?;
    if major != SUPPORTED_MAJOR {
        return Err(ParseError::UnsupportedMajor {
            got: game.schema_version.clone(),
            supported: SUPPORTED_MAJOR,
        });
    }
    Ok(game)
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unparseable schema_version: {0}")]
    BadVersion(String),
    #[error("unsupported schema major version: {got} (this analyzer supports {supported}.x)")]
    UnsupportedMajor { got: String, supported: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimum valid document.
    #[test]
    fn parses_minimal() {
        let json = r#"{
            "schema_version": "1.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": []
        }"#;
        let game = parse_normalized(json).expect("should parse");
        assert_eq!(game.schema_version, "1.0");
        assert_eq!(game.source, "test");
        assert!(game.tournaments.is_empty());
    }

    /// Rejects unknown major versions.
    #[test]
    fn rejects_unknown_major() {
        let json = r#"{
            "schema_version": "2.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": []
        }"#;
        match parse_normalized(json) {
            Err(ParseError::UnsupportedMajor { .. }) => {}
            other => panic!("expected UnsupportedMajor, got {:?}", other),
        }
    }

    /// Accepts a future minor version under the current major.
    #[test]
    fn accepts_minor_bump() {
        let json = r#"{
            "schema_version": "1.7",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": []
        }"#;
        assert!(parse_normalized(json).is_ok());
    }

    /// Optional fields default to absent.
    #[test]
    fn parses_full_board() {
        let json = r#"{
            "schema_version": "1.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": [{
                "name": "Demo",
                "events": [{
                    "date": "2026-04-25",
                    "sessions": [{
                        "session_number": 1,
                        "boards": [{
                            "number": 1,
                            "dealer": "N",
                            "vulnerability": "None",
                            "deal": {
                                "N": {"S": ["A","K"], "H": [], "D": [], "C": []},
                                "E": {"S": [], "H": [], "D": [], "C": []},
                                "S": {"S": [], "H": [], "D": [], "C": []},
                                "W": {"S": [], "H": [], "D": [], "C": []}
                            },
                            "double_dummy": {
                                "N": {"C": 4, "D": 1, "H": 3, "S": 5, "NT": 5},
                                "S": {"C": 4, "D": 1, "H": 3, "S": 5, "NT": 5},
                                "E": {"C": 2, "D": 6, "H": 3, "S": 2, "NT": 2},
                                "W": {"C": 2, "D": 6, "H": 3, "S": 2, "NT": 2}
                            },
                            "par": [
                                {"score": 420, "contract": "4H", "declarer": "N"},
                                {"score": 420, "contract": "4S", "declarer": "S"}
                            ],
                            "results": []
                        }]
                    }]
                }]
            }]
        }"#;
        let game = parse_normalized(json).expect("should parse");
        let board = &game.tournaments[0].events[0].sessions[0].boards[0];
        assert_eq!(board.number, 1);
        assert_eq!(board.par.len(), 2);
        assert_eq!(board.par[0].contract, "4H");
        let dd = board.double_dummy.as_ref().unwrap();
        assert_eq!(dd.north.as_ref().unwrap().spades, Some(5));
    }

    /// Bare `tricks` field can be omitted (null for score-only rows).
    #[test]
    fn result_tricks_optional() {
        let json = r#"{
            "contract": "4S",
            "declarer": "N",
            "score": 420,
            "ns_pair": {"players": [{"name": "A"}, {"name": "B"}]},
            "ew_pair": {"players": [{"name": "C"}, {"name": "D"}]}
        }"#;
        let r: Result = serde_json::from_str(json).expect("should parse");
        assert!(r.tricks.is_none());
        assert_eq!(r.contract, Some("4S".into()));
    }
}
