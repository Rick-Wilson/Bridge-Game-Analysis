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
    let player_lookup = build_player_lookup(&bws_data);

    // Build pair-number-to-players lookup (handles Howell pair numbering)
    let pair_lookup = build_pair_lookup(&bws_data, &player_lookup);

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
            .get(&(received.section, ns_pair_num))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&player_lookup, received.section, ns_pair_num, "NS")
            });
        let (e_name, w_name) = pair_lookup
            .get(&(received.section, ew_pair_num))
            .cloned()
            .unwrap_or_else(|| {
                resolve_pair_from_table(&player_lookup, received.section, ew_pair_num, "EW")
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

/// Build a pair-number-to-players mapping for Howell movements.
///
/// In a Howell, pairs switch between NS and EW, so pair numbers don't
/// correspond to a fixed direction. The standard numbering is:
/// - Pairs 1..N = NS starters at tables 1..N
/// - Pairs N+1..2N = EW starters at tables 1..N
///
/// This is only built when pair numbers exceed the number of tables
/// (indicating a Howell). For Mitchell, the caller falls back to
/// direct table+direction lookup.
fn build_pair_lookup(
    bws_data: &BwsData,
    player_lookup: &HashMap<(i32, i32, String), String>,
) -> HashMap<(i32, i32), (String, String)> {
    // Find max table and max pair number per section
    let mut max_table_by_section: HashMap<i32, i32> = HashMap::new();
    for pn in &bws_data.player_numbers {
        let entry = max_table_by_section.entry(pn.section).or_insert(0);
        *entry = (*entry).max(pn.table);
    }

    let mut max_pair_by_section: HashMap<i32, i32> = HashMap::new();
    for rd in &bws_data.received_data {
        let entry = max_pair_by_section.entry(rd.section).or_insert(0);
        *entry = (*entry).max(rd.pair_ns).max(rd.pair_ew);
    }

    let mut pair_lookup: HashMap<(i32, i32), (String, String)> = HashMap::new();

    for (&section, &num_tables) in &max_table_by_section {
        let max_pair = max_pair_by_section.get(&section).copied().unwrap_or(0);
        if max_pair <= num_tables {
            // Mitchell — pair numbers = table numbers, handled by fallback
            continue;
        }

        // Howell: pairs 1..N = NS at tables 1..N, pairs N+1..2N = EW at tables 1..N
        for table in 1..=num_tables {
            let n = player_lookup.get(&(section, table, "N".to_string()));
            let s = player_lookup.get(&(section, table, "S".to_string()));
            if let (Some(n), Some(s)) = (n, s) {
                pair_lookup.insert((section, table), (n.clone(), s.clone()));
            }

            let e = player_lookup.get(&(section, table, "E".to_string()));
            let w = player_lookup.get(&(section, table, "W".to_string()));
            if let (Some(e), Some(w)) = (e, w) {
                pair_lookup.insert((section, table + num_tables), (e.clone(), w.clone()));
            }
        }
    }

    pair_lookup
}

/// Fallback for Mitchell movements: look up players by table number and direction.
fn resolve_pair_from_table(
    player_lookup: &HashMap<(i32, i32, String), String>,
    section: i32,
    table: i32,
    dir: &str,
) -> (String, String) {
    let (d1, d2) = match dir {
        "NS" => ("N", "S"),
        _ => ("E", "W"),
    };
    let p1 = player_lookup
        .get(&(section, table, d1.to_string()))
        .cloned()
        .unwrap_or_else(|| format!("Player {}-{}", d1, table));
    let p2 = player_lookup
        .get(&(section, table, d2.to_string()))
        .cloned()
        .unwrap_or_else(|| format!("Player {}-{}", d2, table));
    (p1, p2)
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
