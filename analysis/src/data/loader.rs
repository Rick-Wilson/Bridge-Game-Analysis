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

    // Build player lookup from PlayerNumbers table
    // This maps (section, table, direction) -> player name
    let player_lookup = build_player_lookup(&bws_data);

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

        // In pairs games with movement, NS and EW pairs come from different home tables
        // pair_ns = the table number where the NS pair is stationed
        // pair_ew = the table number where the EW pair is stationed
        let ns_table = received.pair_ns;
        let ew_table = received.pair_ew;

        // Get player names - use placeholder if missing from BWS
        let n_name = player_lookup
            .get(&(received.section, ns_table, "N".to_string()))
            .cloned()
            .unwrap_or_else(|| format!("Player N-{}", ns_table));
        let s_name = player_lookup
            .get(&(received.section, ns_table, "S".to_string()))
            .cloned()
            .unwrap_or_else(|| format!("Player S-{}", ns_table));
        let e_name = player_lookup
            .get(&(received.section, ew_table, "E".to_string()))
            .cloned()
            .unwrap_or_else(|| format!("Player E-{}", ew_table));
        let w_name = player_lookup
            .get(&(received.section, ew_table, "W".to_string()))
            .cloned()
            .unwrap_or_else(|| format!("Player W-{}", ew_table));

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

/// Build a lookup from (section, table, direction) to player name
fn build_player_lookup(bws_data: &BwsData) -> HashMap<(i32, i32, String), String> {
    let mut lookup = HashMap::new();

    for pn in &bws_data.player_numbers {
        if let Some(ref name) = pn.name {
            if !name.is_empty() {
                lookup.insert((pn.section, pn.table, pn.direction.clone()), name.clone());
            }
        }
    }

    lookup
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
