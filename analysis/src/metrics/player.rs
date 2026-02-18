use crate::data::{BoardResult, GameData, ParsedContract};
use crate::identity::{normalize_name, Partnership, PartnershipDirection, PlayerId};
use std::collections::HashMap;

/// Player's role on a board
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerRole {
    /// Player was declarer
    Declarer,
    /// Player's partner was declarer (dummy)
    Dummy,
    /// Opponent was declarer (defender)
    Defender,
}

/// Cause category for a board result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCause {
    /// Good result from skillful play/defense/bidding
    Good,
    /// Good result from opponent mistake or luck
    Lucky,
    /// Result affected primarily by declarer play
    Play,
    /// Result affected primarily by defense
    Defense,
    /// Result affected primarily by auction/bidding
    Auction,
    /// Bad result from bad luck or opponent's good play
    Unlucky,
}

/// A single board result for a specific player
#[derive(Debug, Clone)]
pub struct PlayerBoardResult {
    pub board_number: u32,
    /// The player's direction (NS or EW)
    pub direction: PartnershipDirection,
    /// The player's specific seat (N/E/S/W) if known
    pub seat: Option<bridge_parsers::Direction>,
    /// Partner's name
    pub partner: PlayerId,
    /// The contract played
    pub contract: Option<ParsedContract>,
    /// Result string (e.g., "4S=", "3NT+1", "2H-2")
    pub result_str: String,
    /// NS score
    pub ns_score: i32,
    /// Player's score (positive = good for player)
    pub player_score: i32,
    /// Matchpoint percentage
    pub matchpoint_pct: f64,
    /// Did this player declare?
    pub was_declarer: bool,
    /// Player's role on this board
    pub role: PlayerRole,
    /// If declared: tricks vs field average (positive = better)
    pub declarer_vs_field: Option<f64>,
    /// Field contract (most common)
    pub field_contract: Option<ParsedContract>,
    /// Did their contract match the field?
    pub matched_field_contract: bool,
    /// Analyzed cause of the result
    pub cause: ResultCause,
    /// Auto-generated notes explaining the cause
    pub notes: String,
}

/// Complete player analysis
#[derive(Debug)]
pub struct PlayerAnalysis {
    pub player: PlayerId,
    pub player_name: String,
    pub boards_played: u32,
    pub boards_declared: u32,
    pub avg_matchpoint_pct: f64,
    /// Matchpoint percentage when declaring
    pub declaring_mp_pct: Option<f64>,
    /// Matchpoint percentage when dummy (partner declaring)
    pub dummy_mp_pct: Option<f64>,
    /// Matchpoint percentage when defending
    pub defending_mp_pct: Option<f64>,
    pub avg_declarer_vs_field: Option<f64>,
    pub field_contract_pct: f64,
    pub board_results: Vec<PlayerBoardResult>,
}

/// Analyze a specific player's performance
pub fn analyze_player(data: &GameData, player_name: &str) -> Option<PlayerAnalysis> {
    let normalized = normalize_name(player_name);

    // Find the player
    let player_id = data.players.all_players()
        .find(|p| p.id.canonical_name == normalized ||
                  p.id.canonical_name.contains(&normalized))
        .map(|p| p.id.clone())?;

    let display_name = data.players.get(&player_id)
        .map(|p| p.display_name())
        .unwrap_or_else(|| player_id.display_name());

    // Find all results where this player participated
    let player_results: Vec<&BoardResult> = data.results.iter()
        .filter(|r| r.ns_pair.contains(&player_id) || r.ew_pair.contains(&player_id))
        .collect();

    if player_results.is_empty() {
        return None;
    }

    // Compute field contracts and field averages
    let field_contracts = compute_field_contracts(data);
    let field_trick_averages = compute_field_trick_averages(data);

    // Build board-by-board analysis
    let mut board_results = Vec::new();
    let mut total_mp = 0.0;
    let mut total_declarer_diff = 0.0;
    let mut declarer_count = 0;
    let mut field_match_count = 0;

    // Track matchpoints by role
    let mut declaring_mp_total = 0.0;
    let mut declaring_count = 0;
    let mut dummy_mp_total = 0.0;
    let mut dummy_count = 0;
    let mut defending_mp_total = 0.0;
    let mut defending_count = 0;

    for result in &player_results {
        // Determine player's direction and partner
        let (direction, partner) = if result.ns_pair.contains(&player_id) {
            let partner = if result.ns_pair.player1 == player_id {
                result.ns_pair.player2.clone()
            } else {
                result.ns_pair.player1.clone()
            };
            (PartnershipDirection::NorthSouth, partner)
        } else {
            let partner = if result.ew_pair.player1 == player_id {
                result.ew_pair.player2.clone()
            } else {
                result.ew_pair.player1.clone()
            };
            (PartnershipDirection::EastWest, partner)
        };

        // Calculate player's score (from their perspective)
        let player_score = match direction {
            PartnershipDirection::NorthSouth => result.ns_score,
            PartnershipDirection::EastWest => -result.ns_score,
        };

        // Calculate matchpoints
        let all_board_results = data.results_for_board(result.board_number);
        let matchpoint_pct = calculate_matchpoint_pct(result, &all_board_results, direction);
        total_mp += matchpoint_pct;

        // Build result string
        let result_str = build_result_string(result);

        // Check if player declared
        let was_declarer = result.declarer == player_id;

        // Determine player's role and track matchpoints by role
        let role = if was_declarer {
            PlayerRole::Declarer
        } else if result.declaring_direction() == direction {
            // Partner declared (player is dummy)
            PlayerRole::Dummy
        } else {
            // Opponent declared (player is defender)
            PlayerRole::Defender
        };

        // Determine player's specific seat (N/E/S/W) when possible
        let seat = if was_declarer {
            // Player declared, so we know their exact seat
            Some(result.declarer_direction)
        } else if role == PlayerRole::Dummy {
            // Partner declared, player is in the partner seat
            Some(partner_direction(result.declarer_direction))
        } else {
            // Defender - we don't know which specific seat without more info
            None
        };

        match role {
            PlayerRole::Declarer => {
                declaring_mp_total += matchpoint_pct;
                declaring_count += 1;
            }
            PlayerRole::Dummy => {
                dummy_mp_total += matchpoint_pct;
                dummy_count += 1;
            }
            PlayerRole::Defender => {
                defending_mp_total += matchpoint_pct;
                defending_count += 1;
            }
        }

        // Calculate declarer vs field if applicable
        let declarer_vs_field = if was_declarer {
            if let (Some(contract), Some(tricks)) = (&result.contract, result.tricks_made()) {
                let key = format!("{}_{}", result.board_number, strain_key(contract));
                if let Some(&(avg, count)) = field_trick_averages.get(&key) {
                    if count > 1 {
                        let diff = tricks as f64 - avg;
                        total_declarer_diff += diff;
                        declarer_count += 1;
                        Some(diff)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Get field contract
        let field_contract = field_contracts.get(&result.board_number).cloned();

        // Check if matched field contract
        let matched_field = if let (Some(actual), Some(field)) = (&result.contract, &field_contract) {
            actual.level == field.level && strain_key(actual) == strain_key(field)
        } else {
            false
        };
        if matched_field {
            field_match_count += 1;
        }

        // Determine cause and notes for this result
        let (cause, notes) = determine_cause_and_notes(
            role,
            matchpoint_pct,
            declarer_vs_field,
            matched_field,
            &result.contract,
            &field_contract,
        );

        board_results.push(PlayerBoardResult {
            board_number: result.board_number,
            direction,
            seat,
            partner,
            contract: result.contract.clone(),
            result_str,
            ns_score: result.ns_score,
            player_score,
            matchpoint_pct,
            was_declarer,
            role,
            declarer_vs_field,
            field_contract,
            matched_field_contract: matched_field,
            cause,
            notes,
        });
    }

    // Sort by board number
    board_results.sort_by_key(|r| r.board_number);

    let boards_played = board_results.len() as u32;
    let avg_matchpoint_pct = if boards_played > 0 {
        total_mp / boards_played as f64
    } else {
        0.0
    };

    let avg_declarer_vs_field = if declarer_count > 0 {
        Some(total_declarer_diff / declarer_count as f64)
    } else {
        None
    };

    let field_contract_pct = if boards_played > 0 {
        (field_match_count as f64 / boards_played as f64) * 100.0
    } else {
        0.0
    };

    // Calculate role-based matchpoint averages
    let declaring_mp_pct = if declaring_count > 0 {
        Some(declaring_mp_total / declaring_count as f64)
    } else {
        None
    };
    let dummy_mp_pct = if dummy_count > 0 {
        Some(dummy_mp_total / dummy_count as f64)
    } else {
        None
    };
    let defending_mp_pct = if defending_count > 0 {
        Some(defending_mp_total / defending_count as f64)
    } else {
        None
    };

    Some(PlayerAnalysis {
        player: player_id,
        player_name: display_name,
        boards_played,
        boards_declared: declarer_count as u32,
        avg_matchpoint_pct,
        declaring_mp_pct,
        dummy_mp_pct,
        defending_mp_pct,
        avg_declarer_vs_field,
        field_contract_pct,
        board_results,
    })
}

/// Calculate matchpoint percentage for a result
fn calculate_matchpoint_pct(
    result: &BoardResult,
    all_results: &[&BoardResult],
    player_direction: PartnershipDirection,
) -> f64 {
    if all_results.len() <= 1 {
        return 50.0; // Average if only one result
    }

    let player_score = match player_direction {
        PartnershipDirection::NorthSouth => result.ns_score,
        PartnershipDirection::EastWest => -result.ns_score,
    };

    let mut wins = 0.0;
    let mut comparisons = 0;

    for other in all_results {
        if std::ptr::eq(*other, result) {
            continue;
        }

        let other_score = match player_direction {
            PartnershipDirection::NorthSouth => other.ns_score,
            PartnershipDirection::EastWest => -other.ns_score,
        };

        comparisons += 1;
        if player_score > other_score {
            wins += 1.0;
        } else if player_score == other_score {
            wins += 0.5;
        }
    }

    if comparisons > 0 {
        (wins / comparisons as f64) * 100.0
    } else {
        50.0
    }
}

/// Build a result string like "4SN=" or "3NTE+1" (includes declarer direction)
fn build_result_string(result: &BoardResult) -> String {
    use bridge_parsers::Direction;

    let declarer_char = match result.declarer_direction {
        Direction::North => 'N',
        Direction::East => 'E',
        Direction::South => 'S',
        Direction::West => 'W',
    };

    match (&result.contract, result.tricks_relative) {
        (Some(contract), Some(rel)) => {
            let contract_str = contract.display();
            if rel == 0 {
                format!("{}{}=", contract_str, declarer_char)
            } else if rel > 0 {
                format!("{}{}+{}", contract_str, declarer_char, rel)
            } else {
                format!("{}{}{}", contract_str, declarer_char, rel)
            }
        }
        (Some(contract), None) => format!("{}{}", contract.display(), declarer_char),
        (None, _) => "Pass".to_string(),
    }
}

/// Determine the cause and notes for a board result
fn determine_cause_and_notes(
    role: PlayerRole,
    matchpoint_pct: f64,
    declarer_vs_field: Option<f64>,
    matched_field_contract: bool,
    contract: &Option<ParsedContract>,
    field_contract: &Option<ParsedContract>,
) -> (ResultCause, String) {
    let is_good_result = matchpoint_pct >= 55.0;
    let is_bad_result = matchpoint_pct <= 45.0;

    match role {
        PlayerRole::Declarer => {
            // Player was declarer
            if matched_field_contract {
                // Bid the field contract - result depends on play
                if let Some(diff) = declarer_vs_field {
                    let tricks_diff = diff.round() as i32;
                    if tricks_diff < 0 {
                        // Took fewer tricks than field
                        let note = format!("{} {} fewer", -tricks_diff, tricks_word(tricks_diff));
                        return (ResultCause::Play, note);
                    } else if tricks_diff > 0 {
                        // Took more tricks than field
                        let note = format!("+{} {}", tricks_diff, tricks_word(tricks_diff));
                        if is_good_result {
                            return (ResultCause::Good, note);
                        } else {
                            return (ResultCause::Play, note);
                        }
                    } else {
                        // Matched field exactly
                        if is_good_result {
                            return (ResultCause::Good, String::new());
                        } else if is_bad_result {
                            return (ResultCause::Unlucky, "field avg".to_string());
                        }
                    }
                }
            } else {
                // Didn't bid field contract - auction issue
                let note = format_auction_note(contract, field_contract);
                if is_bad_result {
                    return (ResultCause::Auction, note);
                } else if is_good_result {
                    // Good result despite different contract
                    return (ResultCause::Good, note);
                } else {
                    return (ResultCause::Auction, note);
                }
            }
        }
        PlayerRole::Dummy => {
            // Partner was declarer
            if matched_field_contract {
                if let Some(diff) = declarer_vs_field {
                    let tricks_diff = diff.round() as i32;
                    if tricks_diff < 0 {
                        let note = format!("pard {} {}", tricks_diff, tricks_word(tricks_diff));
                        return (ResultCause::Play, note);
                    } else if tricks_diff > 0 {
                        let note = format!("pard +{} {}", tricks_diff, tricks_word(tricks_diff));
                        if is_good_result {
                            return (ResultCause::Good, note);
                        }
                        return (ResultCause::Play, note);
                    }
                }
            } else {
                // Partner didn't bid field contract
                let note = format_auction_note(contract, field_contract);
                if is_bad_result {
                    return (ResultCause::Auction, note);
                } else if is_good_result {
                    return (ResultCause::Good, note);
                }
                return (ResultCause::Auction, note);
            }
        }
        PlayerRole::Defender => {
            // Opponent was declarer
            if let Some(diff) = declarer_vs_field {
                // From defender's perspective, negative diff (declarer took fewer) is good
                let tricks_diff = diff.round() as i32;
                if tricks_diff > 0 {
                    // Declarer took more tricks - bad defense
                    let note = format!("gave {} {}", tricks_diff, tricks_word(tricks_diff));
                    if is_bad_result {
                        return (ResultCause::Defense, note);
                    }
                } else if tricks_diff < 0 {
                    // Declarer took fewer tricks - good defense
                    let note = format!("held to {}", tricks_diff);
                    if is_good_result {
                        return (ResultCause::Good, note);
                    }
                    return (ResultCause::Defense, note);
                }
            }
            // Defender result with no play data
            if is_good_result {
                return (ResultCause::Lucky, String::new());
            } else if is_bad_result {
                return (ResultCause::Unlucky, String::new());
            }
        }
    }

    // Default: categorize by matchpoint result
    if is_good_result {
        (ResultCause::Good, String::new())
    } else if is_bad_result {
        (ResultCause::Unlucky, String::new())
    } else {
        (ResultCause::Good, String::new()) // Neutral result
    }
}

/// Format a note about auction differences
fn format_auction_note(
    contract: &Option<ParsedContract>,
    field_contract: &Option<ParsedContract>,
) -> String {
    match (contract, field_contract) {
        (Some(actual), Some(field)) => {
            let actual_level = actual.level;
            let field_level = field.level;
            let actual_strain = strain_key(actual);
            let field_strain = strain_key(field);

            if actual_strain != field_strain {
                // Different strain
                format!("{} vs {}", actual_strain, field_strain)
            } else if actual_level < field_level {
                // Underbid
                format!("underbid ({})", field.display())
            } else {
                // Overbid
                format!("overbid ({})", field.display())
            }
        }
        (Some(_), None) => String::new(),
        (None, Some(field)) => format!("missed {}", field.display()),
        (None, None) => String::new(),
    }
}

/// Compute field contracts (most common) for each board
fn compute_field_contracts(data: &GameData) -> HashMap<u32, ParsedContract> {
    let mut by_board: HashMap<u32, HashMap<String, (ParsedContract, usize)>> = HashMap::new();

    for result in &data.results {
        if let Some(contract) = &result.contract {
            let key = contract.display();
            by_board
                .entry(result.board_number)
                .or_default()
                .entry(key)
                .or_insert_with(|| (contract.clone(), 0))
                .1 += 1;
        }
    }

    by_board
        .into_iter()
        .filter_map(|(board, contracts)| {
            contracts
                .into_iter()
                .max_by_key(|(_, (_, count))| *count)
                .map(|(_, (contract, _))| (board, contract))
        })
        .collect()
}

/// Compute field trick averages for each (board, strain) combination
fn compute_field_trick_averages(data: &GameData) -> HashMap<String, (f64, usize)> {
    let mut grouped: HashMap<String, Vec<u8>> = HashMap::new();

    for result in &data.results {
        if let (Some(contract), Some(tricks)) = (&result.contract, result.tricks_made()) {
            let key = format!("{}_{}", result.board_number, strain_key(contract));
            grouped.entry(key).or_default().push(tricks);
        }
    }

    grouped
        .into_iter()
        .map(|(key, tricks)| {
            let count = tricks.len();
            let sum: u32 = tricks.iter().map(|&t| t as u32).sum();
            let avg = sum as f64 / count as f64;
            (key, (avg, count))
        })
        .collect()
}

/// Get strain key for a contract
fn strain_key(contract: &ParsedContract) -> &'static str {
    use bridge_parsers::Strain;
    match contract.strain {
        Strain::Clubs => "C",
        Strain::Diamonds => "D",
        Strain::Hearts => "H",
        Strain::Spades => "S",
        Strain::NoTrump => "NT",
    }
}

/// Pluralize "trick" based on count
fn tricks_word(n: i32) -> &'static str {
    if n.abs() == 1 { "trick" } else { "tricks" }
}

/// Get the partner direction (opposite seat in same partnership)
fn partner_direction(dir: bridge_parsers::Direction) -> bridge_parsers::Direction {
    use bridge_parsers::Direction;
    match dir {
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::East => Direction::West,
        Direction::West => Direction::East,
    }
}
