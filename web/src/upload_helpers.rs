//! Schema-walking helpers for the upload-response shaping.
//!
//! The (now-retired) Rust analyzer provided a rich `GameData` with a
//! `PlayerRegistry` that handled name normalization + ACBL-number-based
//! deduplication; the upload handlers picked off counts, distinct player
//! lists, missing-name placeholders, and a pair-number index from that.
//!
//! With the analyzer gone, those derived fields all come from a direct
//! walk over the normalized JSON schema. This module is the single home
//! for that walk so the upload-response shape stays identical to the
//! pre-refactor contract (the SPA still expects `players`, `player_acbl`,
//! `missing_players`, `pair_acbl`, etc.).
//!
//! Player identity dedup mirrors the JS side's `playerKey()` in the SPA:
//! ACBL number when present, normalized name otherwise.
//!
//! All functions here work over the schema types directly. No bridge-
//! domain analysis happens — that's the JS port's job.

use parse_files::data::schema::{NormalizedGame, Pair, Player, Session};
use std::collections::{HashMap, HashSet};

use crate::responses::MissingPlayerInfo;

/// Lowercase + trim a player's name to a stable dedup key.
pub fn normalize_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Title-case a player's name for display.
pub fn display_name(name: &str) -> String {
    normalize_name(name)
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Identity key: ACBL number when present, else normalized name. Mirrors
/// the SPA's `playerKey()` in [`web/static/index.html`].
fn player_key(player: &Player) -> String {
    if let Some(acbl) = &player.acbl_id {
        format!("acbl:{}", acbl)
    } else {
        format!("name:{}", normalize_name(&player.name))
    }
}

/// Flattened session view for indexing. Each tournament/event/session
/// triple in the schema becomes one entry, with `session_idx` matching
/// the flattened position used by the SPA's session selector.
pub struct FlatSession<'a> {
    pub session_idx: u32,
    pub label: String,
    pub event_name: Option<String>,
    pub event_date: Option<String>,
    pub session: &'a Session,
}

/// Walk every (tournament, event, session) triple and yield FlatSession
/// values with the right session_idx and a human-readable label.
pub fn flatten_sessions(game: &NormalizedGame) -> Vec<FlatSession<'_>> {
    let mut out = Vec::new();
    let mut session_idx: u32 = 0;
    for tournament in &game.tournaments {
        for event in &tournament.events {
            let multi_session = event.sessions.len() > 1;
            for session in &event.sessions {
                let label = build_label(
                    tournament.name.as_deref(),
                    event.name.as_deref(),
                    event.date.as_deref(),
                    if multi_session {
                        Some(session.session_number)
                    } else {
                        None
                    },
                );
                out.push(FlatSession {
                    session_idx,
                    label,
                    event_name: event.name.clone().or_else(|| tournament.name.clone()),
                    event_date: event.date.clone(),
                    session,
                });
                session_idx += 1;
            }
        }
    }
    out
}

/// Format a session label combining event/tournament/date/session-number.
fn build_label(
    tournament_name: Option<&str>,
    event_name: Option<&str>,
    event_date: Option<&str>,
    session_number: Option<u32>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = event_name.filter(|s| !s.is_empty()) {
        parts.push(s.to_string());
    } else if let Some(s) = tournament_name.filter(|s| !s.is_empty()) {
        parts.push(s.to_string());
    }
    if let Some(d) = event_date.filter(|s| !s.is_empty()) {
        parts.push(d.to_string());
    }
    if let Some(n) = session_number {
        parts.push(format!("Session {}", n));
    }
    if parts.is_empty() {
        "Game".into()
    } else {
        parts.join(" · ")
    }
}

/// Total result count across all boards in a session.
pub fn result_count(session: &Session) -> usize {
    session.boards.iter().map(|b| b.results.len()).sum()
}

/// Distinct, sorted board numbers for a session — only boards that had at
/// least one result played at some table. Boards with no results aren't
/// analyzable so the SPA's board grid shouldn't list them.
pub fn board_numbers(session: &Session) -> Vec<u32> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out: Vec<u32> = Vec::new();
    for board in &session.boards {
        if board.results.is_empty() {
            continue;
        }
        if seen.insert(board.number) {
            out.push(board.number);
        }
    }
    out.sort();
    out
}

/// Aggregated player-related fields the upload response wants.
pub struct PlayerSummary {
    pub display_names: Vec<String>,
    pub player_acbl: HashMap<String, String>,
    pub missing_players: Vec<MissingPlayerInfo>,
    pub pair_acbl: HashMap<String, Vec<Option<String>>>,
}

/// Walk a session's results and aggregate player info for the upload
/// response. Dedups by (acbl_number OR normalized_name); placeholder
/// "Player N-1"-style names go into missing_players. The pair_acbl map
/// is keyed by the pair number (as a string for JSON compatibility) and
/// holds the raw ACBL numbers for both seats — used by the SPA's
/// paste-names dialog to match a roster paste against the upload.
pub fn summarize_players(session: &Session) -> PlayerSummary {
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut display_names: Vec<String> = Vec::new();
    let mut player_acbl: HashMap<String, String> = HashMap::new();
    let mut missing_set: HashSet<String> = HashSet::new();
    let mut missing_players: Vec<MissingPlayerInfo> = Vec::new();
    let mut pair_acbl: HashMap<String, Vec<Option<String>>> = HashMap::new();

    for board in &session.boards {
        for result in &board.results {
            for pair in [&result.ns_pair, &result.ew_pair] {
                record_pair_acbl(pair, &mut pair_acbl);
                for p in &pair.players {
                    let key = player_key(p);
                    if !seen_keys.insert(key) {
                        continue;
                    }
                    let dn = display_name(&p.name);
                    if dn.is_empty() {
                        continue;
                    }
                    if dn.starts_with("Player ") {
                        if missing_set.insert(dn.clone()) {
                            missing_players.push(MissingPlayerInfo {
                                display_name: dn.clone(),
                                acbl_number: p.acbl_id.clone(),
                            });
                        }
                    } else if let Some(acbl) = &p.acbl_id {
                        player_acbl.insert(dn.clone(), acbl.clone());
                    }
                    display_names.push(dn);
                }
            }
        }
    }
    display_names.sort();
    display_names.dedup();
    missing_players.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    PlayerSummary {
        display_names,
        player_acbl,
        missing_players,
        pair_acbl,
    }
}

fn record_pair_acbl(pair: &Pair, out: &mut HashMap<String, Vec<Option<String>>>) {
    let Some(num) = pair.number else {
        return;
    };
    if num <= 0 || pair.players.len() < 2 {
        return;
    }
    out.entry(num.to_string()).or_insert_with(|| {
        vec![
            pair.players[0].acbl_id.clone(),
            pair.players[1].acbl_id.clone(),
        ]
    });
}

/// Apply name overrides (acbl_id → display name) to every player in the
/// session JSON in place. Returns the number of distinct ACBL ids that
/// resulted in at least one rename. Used by /api/names so the next read
/// of /api/normalized reflects user-supplied names.
pub fn apply_name_overrides(
    game: &mut NormalizedGame,
    overrides: &HashMap<String, String>,
) -> usize {
    let mut applied: HashSet<String> = HashSet::new();
    for tournament in &mut game.tournaments {
        for event in &mut tournament.events {
            for session in &mut event.sessions {
                for board in &mut session.boards {
                    for result in &mut board.results {
                        for pair in [&mut result.ns_pair, &mut result.ew_pair] {
                            for p in &mut pair.players {
                                if let Some(acbl) = &p.acbl_id {
                                    if let Some(new_name) = overrides.get(acbl) {
                                        if &p.name != new_name {
                                            p.name = new_name.clone();
                                            applied.insert(acbl.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    applied.len()
}
