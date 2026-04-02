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
    merge_data(bws_data, pbn_boards)
}

/// Merge BWS and PBN data into GameData
fn merge_data(bws_data: BwsData, pbn_boards: Vec<Board>) -> Result<GameData> {
    let mut game_data = GameData::new();

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

    // Build pair-number-to-players lookup using RoundData (handles all movements)
    let pair_lookup = build_pair_lookup(&bws_data);

    // Build ACBL number lookup from PlayerNames table
    let acbl_lookup = build_acbl_lookup(&bws_data);

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

        let (n_name, s_name) = pair_lookup
            .get(&(received.section, ns_pair_num, true))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&bws_data, received.section, ns_pair_num, "NS")
            });
        let (e_name, w_name) = pair_lookup
            .get(&(received.section, ew_pair_num, false))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&bws_data, received.section, ew_pair_num, "EW")
            });

        // Get ACBL numbers if available
        let n_acbl = acbl_lookup.get(&n_name).cloned();
        let s_acbl = acbl_lookup.get(&s_name).cloned();
        let e_acbl = acbl_lookup.get(&e_name).cloned();
        let w_acbl = acbl_lookup.get(&w_name).cloned();

        // Register players
        let n_id = game_data.players.get_or_create(&n_name, n_acbl);
        let s_id = game_data.players.get_or_create(&s_name, s_acbl);
        let e_id = game_data.players.get_or_create(&e_name, e_acbl);
        let w_id = game_data.players.get_or_create(&w_name, w_acbl);

        // Create partnerships with seat-based display ordering
        // NS pairs: North displayed first; EW pairs: West displayed first
        let ns_pair = Partnership::new_seated(n_id.clone(), s_id.clone(), &n_id);
        let ew_pair = Partnership::new_seated(e_id.clone(), w_id.clone(), &w_id);

        if is_passout {
            // Pass-out: score is 0, no declarer or contract
            let result = BoardResult {
                board_number,
                section: received.section,
                table: received.table,
                round: received.round,
                ns_pair,
                ew_pair,
                declarer_direction: Direction::North, // placeholder - not meaningful
                declarer: n_id,                       // placeholder - not meaningful
                contract: None,
                tricks_relative: None,
                ns_score: 0,
                lead_card: None,
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
                section: received.section,
                table: received.table,
                round: received.round,
                ns_pair,
                ew_pair,
                declarer_direction,
                declarer,
                contract,
                tricks_relative,
                ns_score,
                lead_card: received.lead_card.clone(),
            };

            game_data.results.push(result);
        }
    }

    Ok(game_data)
}

/// Build a lookup from (section, pair_number, is_ns) to (player1, player2).
///
/// Uses RoundData round 1 to map pair numbers to physical tables, then
/// PlayerNumbers to get player names. The `is_ns` flag disambiguates
/// Mitchell movements where NS pair 5 and EW pair 5 are different pairs
/// at the same table. In Howell movements, a pair number only appears in
/// one column (NS or EW) per round, but may appear in either column across
/// rounds — we store both keys so lookup works from either column.
fn build_pair_lookup(bws_data: &BwsData) -> HashMap<(i32, i32, bool), (String, String)> {
    // Build raw (section, table, direction) -> name map from PlayerNumbers
    let mut player_at: HashMap<(i32, i32, &str), String> = HashMap::new();
    for pn in &bws_data.player_numbers {
        if let Some(ref name) = pn.name {
            if !name.is_empty() {
                let dir = match pn.direction.as_str() {
                    "N" => "N",
                    "S" => "S",
                    "E" => "E",
                    "W" => "W",
                    _ => continue,
                };
                player_at.insert((pn.section, pn.table, dir), name.clone());
            }
        }
    }

    let mut pair_lookup: HashMap<(i32, i32, bool), (String, String)> = HashMap::new();

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
                let names = (p1.clone(), p2.clone());
                // Store under is_ns=true (always valid for round 1)
                pair_lookup.insert((rd.section, rd.ns_pair, true), names.clone());
                // Also store under is_ns=false for Howell, where this pair
                // may appear as EW in later rounds. Won't collide in Mitchell
                // because Mitchell NS pair N ≠ EW pair N (different players).
                // Actually in Mitchell they DO collide, so only store the
                // false key if the pair numbers differ (Howell indicator).
                if rd.ns_pair != rd.ew_pair {
                    pair_lookup.insert((rd.section, rd.ns_pair, false), names);
                }
            }
        }

        // EW pair at this table in round 1 → E/W players
        if rd.ew_pair > 0 {
            let p1 = player_at.get(&(rd.section, rd.table, "E"));
            let p2 = player_at.get(&(rd.section, rd.table, "W"));
            if let (Some(p1), Some(p2)) = (p1, p2) {
                let names = (p1.clone(), p2.clone());
                pair_lookup.insert((rd.section, rd.ew_pair, false), names.clone());
                if rd.ns_pair != rd.ew_pair {
                    pair_lookup.insert((rd.section, rd.ew_pair, true), names);
                }
            }
        }
    }

    pair_lookup
}

/// Fallback for when pair_lookup has no entry: look up by table + direction.
/// Used for Mitchell movements where pair number = table number.
fn resolve_pair_from_table(
    bws_data: &BwsData,
    section: i32,
    table: i32,
    dir: &str,
) -> (String, String) {
    let (d1, d2) = match dir {
        "NS" => ("N", "S"),
        _ => ("E", "W"),
    };
    let p1 = bws_data
        .get_player_at(section, table, d1)
        .unwrap_or_default();
    let p2 = bws_data
        .get_player_at(section, table, d2)
        .unwrap_or_default();
    let name1 = if p1.is_empty() {
        format!("Player {}-{}", d1, table)
    } else {
        p1.to_string()
    };
    let name2 = if p2.is_empty() {
        format!("Player {}-{}", d2, table)
    } else {
        p2.to_string()
    };
    (name1, name2)
}

/// Build a lookup from player name to ACBL number
fn build_acbl_lookup(bws_data: &BwsData) -> HashMap<String, String> {
    let mut lookup = HashMap::new();

    for pn in &bws_data.player_names {
        if !pn.str_id.is_empty() {
            lookup.insert(pn.name.clone(), pn.str_id.clone());
        }
    }

    lookup
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
