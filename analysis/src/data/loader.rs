use crate::data::types::{BoardData, BoardResult, GameData, ParsedContract};
use crate::error::{AnalysisError, Result};
use crate::identity::Partnership;
use bridge_parsers::bws::{read_bws, BwsData};
use bridge_parsers::pbn::read_pbn;
use bridge_parsers::{Board, Contract, Direction};
use std::collections::HashMap;
use std::path::Path;

/// Load complete game data from files
pub fn load_game_data(
    bws_path: &Path,
    pbn_path: Option<&Path>,
    _masterpoints_url: Option<&str>,
) -> Result<GameData> {
    load_game_data_with_overrides(bws_path, pbn_path, None)
}

/// Load game data with optional name overrides (ACBL number -> display name).
/// Overrides are applied during loading so PlayerIds are created with the
/// right names from the start.
pub fn load_game_data_with_overrides(
    bws_path: &Path,
    pbn_path: Option<&Path>,
    name_overrides: Option<&HashMap<String, String>>,
) -> Result<GameData> {
    // 1. Load BWS data (required)
    let bws_data = read_bws(bws_path)?;

    // 2. Load PBN data if provided
    let pbn_boards = if let Some(pbn) = pbn_path {
        let content = std::fs::read_to_string(pbn)?;
        read_pbn(&content).map_err(|e| AnalysisError::PbnParseError(e.to_string()))?
    } else {
        Vec::new()
    };

    // 3. Merge data
    merge_data(bws_data, pbn_boards, name_overrides)
}

/// Merge BWS and PBN data into GameData
fn merge_data(
    bws_data: BwsData,
    pbn_boards: Vec<Board>,
    name_overrides: Option<&HashMap<String, String>>,
) -> Result<GameData> {
    let mut game_data = GameData::new();

    // Extract event info from Session table
    if let Some(session) = bws_data.sessions.first() {
        game_data.event_name = session
            .name
            .as_ref()
            .map(|n: &String| n.trim().to_string())
            .filter(|n: &String| !n.is_empty());
        game_data.event_date = session
            .date
            .as_ref()
            .map(|d: &String| d.trim().to_string())
            .filter(|d: &String| !d.is_empty());
    }

    // Build board data from PBN (has par contract info)
    for board in &pbn_boards {
        if let Some(num) = board.number {
            game_data.boards.insert(num, BoardData::from_board(board));
        }
    }

    // Add any boards from BWS that aren't in PBN
    for board in &bws_data.boards {
        if let Some(num) = board.number {
            game_data
                .boards
                .entry(num)
                .or_insert_with(|| BoardData::from_board(board));
        }
    }

    // Build pair-number-to-players lookup using RoundData (handles all movements).
    // Each entry carries (name, ACBL number) for both players in the pair.
    let pair_lookup = build_pair_lookup(&bws_data);

    // Track (board, section, table, round) keys we've already accepted so we
    // can drop duplicate ReceivedData rows. BWS files sometimes carry the same
    // hand twice (once with a contract, once with `result` only); we keep the
    // first one and skip the rest.
    let mut seen_keys: std::collections::HashSet<(i32, i32, i32, i32)> =
        std::collections::HashSet::new();

    // Process each result
    for received in &bws_data.received_data {
        // Skip only truly empty results (no contract data at all)
        if received.contract.is_empty() {
            continue;
        }

        let board_number = received.board as u32;
        let is_passout = received.contract.to_uppercase() == "PASS";

        // Look up players by pair number. In Howell movements, pair numbers
        // don't correspond 1:1 to table directions — use the pair lookup.
        // For Mitchell, fall back to direct table+direction lookup.
        let ns_pair_num = received.pair_ns;
        let ew_pair_num = received.pair_ew;

        // NS pair: first=N, second=S
        let ((mut n_name, n_acbl), (mut s_name, s_acbl)) = pair_lookup
            .get(&(received.section, ns_pair_num, true))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&bws_data, received.section, ns_pair_num, "NS")
            });
        // EW pair: first=W, second=E (seat-based display ordering)
        let ((mut w_name, w_acbl), (mut e_name, e_acbl)) = pair_lookup
            .get(&(received.section, ew_pair_num, false))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&bws_data, received.section, ew_pair_num, "EW")
            });

        // Apply name overrides: if an ACBL number has an override and the
        // current name is a placeholder, replace it with the overridden name.
        if let Some(overrides) = name_overrides {
            let apply = |name: &mut String, acbl: &Option<String>| {
                if name.starts_with("Player ") {
                    if let Some(acbl_num) = acbl {
                        if let Some(real_name) = overrides.get(acbl_num) {
                            *name = real_name.clone();
                        }
                    }
                }
            };
            apply(&mut n_name, &n_acbl);
            apply(&mut s_name, &s_acbl);
            apply(&mut e_name, &e_acbl);
            apply(&mut w_name, &w_acbl);
        }

        // Register players (ACBL numbers preserved even when name is a placeholder)
        let n_id = game_data.players.get_or_create(&n_name, n_acbl);
        let s_id = game_data.players.get_or_create(&s_name, s_acbl);
        let e_id = game_data.players.get_or_create(&e_name, e_acbl);
        let w_id = game_data.players.get_or_create(&w_name, w_acbl);

        // Track pair number → (first_player, second_player) in display order
        // (N-S for NS pair, W-E for EW pair). Used for name-override paste lookup.
        game_data
            .pairs_by_number
            .entry((received.section, ns_pair_num))
            .or_insert_with(|| (n_id.clone(), s_id.clone()));
        game_data
            .pairs_by_number
            .entry((received.section, ew_pair_num))
            .or_insert_with(|| (w_id.clone(), e_id.clone()));

        // Create partnerships with seat-based display ordering
        // NS pairs: North displayed first; EW pairs: West displayed first
        let ns_pair = Partnership::new_seated(n_id.clone(), s_id.clone(), &n_id);
        let ew_pair = Partnership::new_seated(e_id.clone(), w_id.clone(), &w_id);

        if is_passout {
            // Pass-out: score is 0, no declarer or contract
            let result = BoardResult {
                board_number,
                ns_pair,
                ew_pair,
                declarer_direction: Direction::North, // placeholder - not meaningful
                declarer: n_id,                       // placeholder - not meaningful
                contract: None,
                tricks_relative: None,
                ns_score: 0,
            };
            game_data.results.push(result);
        } else {
            // Parse declarer direction
            let declarer_direction = parse_declarer_direction(received.declarer, &received.ns_ew);

            // Get declarer player ID
            let declarer = match declarer_direction {
                Direction::North => n_id,
                Direction::South => s_id,
                Direction::East => e_id,
                Direction::West => w_id,
            };

            // Parse contract
            let contract = ParsedContract::parse(&received.contract);

            // Parse result
            let tricks_relative = Contract::parse_result(&received.result);

            // Calculate score
            let ns_score = calculate_ns_score(
                &contract,
                tricks_relative,
                declarer_direction,
                board_number,
                &game_data.boards,
            );

            let result = BoardResult {
                board_number,
                ns_pair,
                ew_pair,
                declarer_direction,
                declarer,
                contract,
                tricks_relative,
                ns_score,
            };

            // Deduplicate: skip if we already have this board + section + table + round
            let key = (
                received.board,
                received.section,
                received.table,
                received.round,
            );
            if seen_keys.insert(key) {
                game_data.results.push(result);
            }
        }
    }

    Ok(game_data)
}

/// A resolved player identity (name + ACBL number).
/// Both fields come from PlayerNumbers; ACBL number may be preserved
/// even when the name is empty (falls back to a placeholder name).
type PlayerEntry = (String, Option<String>);

/// Build a lookup from (section, pair_number, is_ns) to
/// ((player1_name, player1_acbl), (player2_name, player2_acbl)).
///
/// Uses RoundData round 1 to map pair numbers to physical tables, then
/// PlayerNumbers to get player names and ACBL numbers. When a seat has
/// an ACBL number but no name, a placeholder like "Player N-3" is used
/// for the name and the ACBL number is still carried through.
fn build_pair_lookup(bws_data: &BwsData) -> HashMap<(i32, i32, bool), (PlayerEntry, PlayerEntry)> {
    // Build raw (section, table, direction) -> (name, acbl) map from PlayerNumbers.
    // Preserves ACBL numbers even when names are empty.
    let mut player_at: HashMap<(i32, i32, &'static str), PlayerEntry> = HashMap::new();
    for pn in &bws_data.player_numbers {
        let dir: &'static str = match pn.direction.as_str() {
            "N" => "N",
            "S" => "S",
            "E" => "E",
            "W" => "W",
            _ => continue,
        };
        let name = pn
            .name
            .as_ref()
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Player {}-{}", dir, pn.table));
        let acbl = if pn.number.is_empty() {
            None
        } else {
            Some(pn.number.clone())
        };
        player_at.insert((pn.section, pn.table, dir), (name, acbl));
    }

    let mut pair_lookup: HashMap<(i32, i32, bool), (PlayerEntry, PlayerEntry)> = HashMap::new();

    if bws_data.round_data.is_empty() {
        return pair_lookup;
    }

    // Use round 1 to establish pair → players mapping.
    for rd in &bws_data.round_data {
        if rd.round != 1 {
            continue;
        }

        // NS pair at this table in round 1 → N/S players
        if rd.ns_pair > 0 {
            let p1 = player_at.get(&(rd.section, rd.table, "N"));
            let p2 = player_at.get(&(rd.section, rd.table, "S"));
            if let (Some(p1), Some(p2)) = (p1, p2) {
                let entries = (p1.clone(), p2.clone());
                pair_lookup.insert((rd.section, rd.ns_pair, true), entries.clone());
                // Also store under is_ns=false for Howell where this pair
                // may appear as EW in later rounds. Skip in Mitchell (where
                // NS pair N == EW pair N and would overwrite the EW pair).
                if rd.ns_pair != rd.ew_pair {
                    pair_lookup.insert((rd.section, rd.ns_pair, false), entries);
                }
            }
        }

        // EW pair at this table in round 1 → E/W players (W displayed first)
        if rd.ew_pair > 0 {
            let p1 = player_at.get(&(rd.section, rd.table, "W"));
            let p2 = player_at.get(&(rd.section, rd.table, "E"));
            if let (Some(p1), Some(p2)) = (p1, p2) {
                let entries = (p1.clone(), p2.clone());
                pair_lookup.insert((rd.section, rd.ew_pair, false), entries.clone());
                if rd.ns_pair != rd.ew_pair {
                    pair_lookup.insert((rd.section, rd.ew_pair, true), entries);
                }
            }
        }
    }

    pair_lookup
}

/// Fallback for when pair_lookup has no entry: look up by table + direction.
/// Used for Mitchell movements where pair number = table number.
/// Returns entries in display order: NS = (N, S), EW = (W, E).
fn resolve_pair_from_table(
    bws_data: &BwsData,
    section: i32,
    table: i32,
    dir: &str,
) -> (PlayerEntry, PlayerEntry) {
    let (d1, d2) = match dir {
        "NS" => ("N", "S"),
        _ => ("W", "E"),
    };
    let lookup_seat = |d: &str| -> PlayerEntry {
        let pn = bws_data
            .player_numbers
            .iter()
            .find(|p| p.section == section && p.table == table && p.direction == d);
        match pn {
            Some(pn) => {
                let name = pn
                    .name
                    .as_ref()
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Player {}-{}", d, table));
                let acbl = if pn.number.is_empty() {
                    None
                } else {
                    Some(pn.number.clone())
                };
                (name, acbl)
            }
            None => (format!("Player {}-{}", d, table), None),
        }
    };
    (lookup_seat(d1), lookup_seat(d2))
}

/// Parse declarer direction from BWS format
fn parse_declarer_direction(_declarer_code: i32, ns_ew: &str) -> Direction {
    // BWS format:
    // - The "Declarer" field contains the PAIR NUMBER of the declaring pair, not a direction code
    // - The "NS/EW" field contains the actual direction: N, S, E, or W
    match ns_ew.trim().to_uppercase().as_str() {
        "N" => Direction::North,
        "S" => Direction::South,
        "E" => Direction::East,
        "W" => Direction::West,
        _ => Direction::North, // Default
    }
}

/// Calculate NS score from contract and result
fn calculate_ns_score(
    contract: &Option<ParsedContract>,
    tricks_relative: Option<i32>,
    declarer: Direction,
    board_number: u32,
    boards: &HashMap<u32, BoardData>,
) -> i32 {
    let contract = match contract {
        Some(c) => c,
        None => return 0,
    };

    let tricks = match tricks_relative {
        Some(t) => t,
        None => return 0,
    };

    // Determine if declarer is vulnerable
    let vulnerable = boards
        .get(&board_number)
        .map(|b| b.is_declarer_vulnerable(declarer))
        .unwrap_or(false);

    // Use bridge_parsers Contract to calculate score
    let bp_contract = bridge_parsers::Contract::new(
        contract.level,
        contract.strain,
        contract.doubled,
        declarer.to_char(),
    );

    let raw_score = bp_contract.score(tricks, vulnerable);

    // Convert to NS score
    match declarer {
        Direction::North | Direction::South => raw_score,
        Direction::East | Direction::West => -raw_score,
    }
}

/// Extension trait for converting Direction to char
#[allow(dead_code)]
pub(crate) trait DirectionExt {
    /// Returns the single-character representation of this direction
    fn to_char(&self) -> char;
}

impl DirectionExt for Direction {
    fn to_char(&self) -> char {
        match self {
            Direction::North => 'N',
            Direction::East => 'E',
            Direction::South => 'S',
            Direction::West => 'W',
        }
    }
}
