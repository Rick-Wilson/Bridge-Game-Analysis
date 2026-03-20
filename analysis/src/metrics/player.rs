use crate::data::{BoardData, BoardResult, GameData, ParsedContract};
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

/// Primary classification of a board's structural dynamics.
///
/// Identifies the single most important characteristic of a board across all
/// tables, used to guide cause analysis and provide context in CLI output.
/// Priority: Competitive > SlamVsGame > GameVsPartscore > StrainChoice > Flat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardType {
    /// Two sides competing in different strains
    Competitive {
        ns_strain: &'static str,
        ew_strain: &'static str,
    },
    /// Some tables reached slam, others stopped in game or below
    SlamVsGame { strain: &'static str },
    /// Results split between game level and part-score in the same strain
    GameVsPartscore { strain: &'static str },
    /// Same side declares in multiple different strains
    StrainChoice {
        strain_a: &'static str,
        strain_b: &'static str,
    },
    /// No significant structural variation
    Flat,
}

impl std::fmt::Display for BoardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardType::Competitive {
                ns_strain,
                ew_strain,
            } => write!(f, "Competitive ({} vs {})", ns_strain, ew_strain),
            BoardType::SlamVsGame { strain } => write!(f, "Slam vs Game ({})", strain),
            BoardType::GameVsPartscore { strain } => {
                write!(f, "Game vs Partscore ({})", strain)
            }
            BoardType::StrainChoice { strain_a, strain_b } => {
                write!(f, "Strain choice ({} vs {})", strain_a, strain_b)
            }
            BoardType::Flat => write!(f, "Flat"),
        }
    }
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
    /// Board type classification
    pub board_type: BoardType,
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

// ==================== Shared Analysis Types ====================

/// Analysis result for one direction on one board result.
///
/// This is the core output of the shared analysis logic, used by both
/// per-player and per-board views.
#[derive(Debug, Clone)]
pub struct DirectionAnalysis {
    /// Matchpoint percentage for this direction
    pub matchpoint_pct: f64,
    /// Role on this board
    pub role: PlayerRole,
    /// Declarer's tricks vs field average (always from declarer perspective)
    pub declarer_vs_field: Option<f64>,
    /// Whether the contract matched the field contract
    pub matched_field_contract: bool,
    /// Whether the contract strain matched the field strain
    pub same_strain_as_field: bool,
    /// Analyzed cause of the result
    pub cause: ResultCause,
    /// Auto-generated notes explaining the cause
    pub notes: String,
}

/// Pre-computed field context for a single board.
///
/// Contains all the "what did the field do?" information needed to
/// evaluate any individual result on this board.
#[derive(Debug)]
pub struct BoardContext {
    /// Most common contract on this board
    pub field_contract: Option<ParsedContract>,
    /// Primary structural classification of this board
    pub board_type: BoardType,
    /// Average tricks by strain key (e.g., "S" -> (avg, count))
    field_trick_averages: HashMap<&'static str, (f64, usize)>,
    /// Competitive info from NS perspective
    competitive_ns: Option<CompetitiveInfo>,
    /// Competitive info from EW perspective
    competitive_ew: Option<CompetitiveInfo>,
    /// Whether NS typically declares this board (majority of non-passout results)
    ns_typically_declares: bool,
    /// Whether EW typically declares this board
    ew_typically_declares: bool,
    /// Direction that typically declares the field contract (None if evenly split)
    field_declaring_direction: Option<PartnershipDirection>,
}

impl BoardContext {
    /// Get competitive info for a direction
    fn competitive_info(&self, direction: PartnershipDirection) -> Option<&CompetitiveInfo> {
        match direction {
            PartnershipDirection::NorthSouth => self.competitive_ns.as_ref(),
            PartnershipDirection::EastWest => self.competitive_ew.as_ref(),
        }
    }

    /// Whether the given direction's side typically declares
    fn player_side_typically_declares(&self, direction: PartnershipDirection) -> bool {
        match direction {
            PartnershipDirection::NorthSouth => self.ns_typically_declares,
            PartnershipDirection::EastWest => self.ew_typically_declares,
        }
    }

    /// Get declarer vs field trick difference for a result
    fn declarer_vs_field(&self, result: &BoardResult) -> Option<f64> {
        let contract = result.contract.as_ref()?;
        let tricks = result.tricks_made()?;
        let skey = strain_key(contract);
        let &(avg, count) = self.field_trick_averages.get(skey)?;
        if count > 1 {
            Some(tricks as f64 - avg)
        } else {
            None
        }
    }
}

/// One table's result for the board view, with analysis for both sides
#[derive(Debug, Clone)]
pub struct BoardTableResult {
    /// NS partnership
    pub ns_pair: Partnership,
    /// EW partnership
    pub ew_pair: Partnership,
    /// Contract played (None if passed out)
    pub contract: Option<ParsedContract>,
    /// Declarer's seat
    pub declarer_direction: bridge_parsers::Direction,
    /// Result string (e.g., "4SN=")
    pub result_str: String,
    /// NS score
    pub ns_score: i32,
    /// Analysis from NS perspective
    pub ns_analysis: DirectionAnalysis,
    /// Analysis from EW perspective
    pub ew_analysis: DirectionAnalysis,
}

/// Complete analysis for a single board across all tables
#[derive(Debug)]
pub struct BoardAnalysis {
    /// Board number
    pub board_number: u32,
    /// Field contract (most common)
    pub field_contract: Option<ParsedContract>,
    /// Primary structural classification
    pub board_type: BoardType,
    /// Results sorted by NS matchpoint % descending
    pub results: Vec<BoardTableResult>,
}

// ==================== Internal Types ====================

/// Competitive board context from the player's perspective.
///
/// On boards where both sides have a clear primary strain (e.g., NS in hearts,
/// EW in spades), the auction dynamics — how high each side pushed — determine
/// the result more than trick-taking or field contract matching.
#[derive(Debug, Clone)]
struct CompetitiveInfo {
    /// Player's side primary strain
    player_strain: bridge_parsers::Strain,
    /// Max level player's side reached in their strain across all tables
    player_max_level: u8,
    /// Opponent's side primary strain
    opp_strain: bridge_parsers::Strain,
    /// Max level opponents reached in their strain across all tables
    opp_max_level: u8,
}

/// Context passed to cause analysis
#[allow(dead_code)]
struct CauseContext<'a> {
    role: PlayerRole,
    matchpoint_pct: f64,
    declarer_vs_field: Option<f64>,
    matched_field_contract: bool,
    same_strain_as_field: bool,
    player_side_typically_declares: bool,
    /// True when the declaring direction at this table differs from the field's
    /// typical declaring direction. Cross-direction means it's a competitive
    /// action — never compare strains across directions.
    field_is_cross_direction: bool,
    /// True when the declarer went down (tricks_relative < 0)
    went_down: bool,
    /// Double-dummy expected tricks for the declarer in this contract's strain/direction
    dd_tricks: Option<u8>,
    /// Actual tricks made by the declarer
    tricks_made: Option<u8>,
    board_type: &'a BoardType,
    competitive: Option<&'a CompetitiveInfo>,
    contract: &'a Option<ParsedContract>,
    field_contract: &'a Option<ParsedContract>,
}

// ==================== Public Analysis Functions ====================

/// Analyze a specific player's performance
pub fn analyze_player(data: &GameData, player_name: &str) -> Option<PlayerAnalysis> {
    let normalized = normalize_name(player_name);

    // Find the player
    let player_id = data
        .players
        .all_players()
        .find(|p| p.id.canonical_name == normalized || p.id.canonical_name.contains(&normalized))
        .map(|p| p.id.clone())?;

    let display_name = data
        .players
        .get(&player_id)
        .map(|p| p.display_name())
        .unwrap_or_else(|| player_id.display_name());

    // Find all results where this player participated
    let player_results: Vec<&BoardResult> = data
        .results
        .iter()
        .filter(|r| r.ns_pair.contains(&player_id) || r.ew_pair.contains(&player_id))
        .collect();

    if player_results.is_empty() {
        return None;
    }

    let mut board_results = Vec::new();
    let mut total_mp = 0.0;
    let mut total_declarer_diff = 0.0;
    let mut declarer_count = 0;
    let mut field_match_count = 0;

    // Track matchpoints by role
    let mut declaring_mp_total = 0.0;
    let mut declaring_board_count = 0;
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

        let player_score = match direction {
            PartnershipDirection::NorthSouth => result.ns_score,
            PartnershipDirection::EastWest => -result.ns_score,
        };

        // Determine player-specific role (distinguishes Declarer from Dummy)
        let is_passout = result.contract.is_none();
        let was_declarer = !is_passout && result.declarer == player_id;
        let role = if is_passout {
            PlayerRole::Defender
        } else if was_declarer {
            PlayerRole::Declarer
        } else if result.declaring_direction() == direction {
            PlayerRole::Dummy
        } else {
            PlayerRole::Defender
        };

        // Determine player's specific seat (N/E/S/W) when possible
        let seat = if is_passout {
            None
        } else if was_declarer {
            Some(result.declarer_direction)
        } else if role == PlayerRole::Dummy {
            Some(partner_direction(result.declarer_direction))
        } else {
            None
        };

        // Compute board context and run shared analysis
        let all_board_results = data.results_for_board(result.board_number);
        let board_data = data.boards.get(&result.board_number);
        let board_ctx = compute_board_context(&all_board_results);
        let analysis = analyze_direction(
            result,
            direction,
            role,
            &board_ctx,
            &all_board_results,
            board_data,
        );

        // Track totals
        total_mp += analysis.matchpoint_pct;

        match role {
            PlayerRole::Declarer => {
                declaring_mp_total += analysis.matchpoint_pct;
                declaring_board_count += 1;
            }
            PlayerRole::Dummy => {
                dummy_mp_total += analysis.matchpoint_pct;
                dummy_count += 1;
            }
            PlayerRole::Defender => {
                defending_mp_total += analysis.matchpoint_pct;
                defending_count += 1;
            }
        }

        if was_declarer {
            if let Some(diff) = analysis.declarer_vs_field {
                total_declarer_diff += diff;
                declarer_count += 1;
            }
        }

        if analysis.matched_field_contract {
            field_match_count += 1;
        }

        let result_str = build_result_string(result);

        board_results.push(PlayerBoardResult {
            board_number: result.board_number,
            direction,
            seat,
            partner,
            contract: result.contract.clone(),
            result_str,
            ns_score: result.ns_score,
            player_score,
            matchpoint_pct: analysis.matchpoint_pct,
            was_declarer,
            role,
            declarer_vs_field: if was_declarer {
                analysis.declarer_vs_field
            } else {
                None
            },
            field_contract: board_ctx.field_contract,
            board_type: board_ctx.board_type,
            matched_field_contract: analysis.matched_field_contract,
            cause: analysis.cause,
            notes: analysis.notes,
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

    let declaring_mp_pct = if declaring_board_count > 0 {
        Some(declaring_mp_total / declaring_board_count as f64)
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

/// Analyze all results for a single board across all tables
pub fn analyze_board(data: &GameData, board_number: u32) -> Option<BoardAnalysis> {
    let all_results = data.results_for_board(board_number);
    if all_results.is_empty() {
        return None;
    }

    let board_data = data.boards.get(&board_number);
    let board_ctx = compute_board_context(&all_results);

    let mut results = Vec::new();
    for result in &all_results {
        let declaring_dir = result.declaring_direction();
        let is_passout = result.contract.is_none();

        // For the board view, the declaring pair gets Declarer role,
        // the other pair gets Defender role (no Dummy distinction at pair level)
        let ns_role = if is_passout {
            PlayerRole::Defender
        } else if declaring_dir == PartnershipDirection::NorthSouth {
            PlayerRole::Declarer
        } else {
            PlayerRole::Defender
        };
        let ew_role = if is_passout {
            PlayerRole::Defender
        } else if declaring_dir == PartnershipDirection::EastWest {
            PlayerRole::Declarer
        } else {
            PlayerRole::Defender
        };

        let ns_analysis = analyze_direction(
            result,
            PartnershipDirection::NorthSouth,
            ns_role,
            &board_ctx,
            &all_results,
            board_data,
        );
        let ew_analysis = analyze_direction(
            result,
            PartnershipDirection::EastWest,
            ew_role,
            &board_ctx,
            &all_results,
            board_data,
        );

        results.push(BoardTableResult {
            ns_pair: result.ns_pair.clone(),
            ew_pair: result.ew_pair.clone(),
            contract: result.contract.clone(),
            declarer_direction: result.declarer_direction,
            result_str: build_result_string(result),
            ns_score: result.ns_score,
            ns_analysis,
            ew_analysis,
        });
    }

    // Sort by NS matchpoint % descending (best NS result first, like ACBL)
    results.sort_by(|a, b| {
        b.ns_analysis
            .matchpoint_pct
            .partial_cmp(&a.ns_analysis.matchpoint_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Some(BoardAnalysis {
        board_number,
        field_contract: board_ctx.field_contract,
        board_type: board_ctx.board_type,
        results,
    })
}

// ==================== Shared Core ====================

/// Compute field-level context for a single board.
///
/// This pre-computes everything about "what the field did" so that
/// individual results can be evaluated efficiently and symmetrically.
pub fn compute_board_context(all_results: &[&BoardResult]) -> BoardContext {
    // Field contract: most common contract
    let field_contract = {
        let mut counts: HashMap<String, (ParsedContract, usize)> = HashMap::new();
        for result in all_results {
            if let Some(contract) = &result.contract {
                let key = contract.display();
                counts.entry(key).or_insert_with(|| (contract.clone(), 0)).1 += 1;
            }
        }
        counts
            .into_values()
            .max_by_key(|(_, count)| *count)
            .map(|(contract, _)| contract)
    };

    // Trick averages by strain
    let field_trick_averages = {
        let mut grouped: HashMap<&'static str, Vec<u8>> = HashMap::new();
        for result in all_results {
            if let (Some(contract), Some(tricks)) = (&result.contract, result.tricks_made()) {
                grouped
                    .entry(strain_key(contract))
                    .or_default()
                    .push(tricks);
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
    };

    // Competitive info for both sides
    let competitive_ns = compute_competitive_info(all_results, PartnershipDirection::NorthSouth);
    let competitive_ew = compute_competitive_info(all_results, PartnershipDirection::EastWest);

    // Which side typically declares
    let non_passout: Vec<_> = all_results
        .iter()
        .filter(|r| r.contract.is_some())
        .collect();
    let ns_declaring = non_passout
        .iter()
        .filter(|r| r.declaring_direction() == PartnershipDirection::NorthSouth)
        .count();
    let ns_typically_declares = !non_passout.is_empty() && ns_declaring * 2 > non_passout.len();
    let ew_typically_declares =
        !non_passout.is_empty() && (non_passout.len() - ns_declaring) * 2 > non_passout.len();

    let field_declaring_direction = if ns_typically_declares {
        Some(PartnershipDirection::NorthSouth)
    } else if ew_typically_declares {
        Some(PartnershipDirection::EastWest)
    } else {
        None
    };

    let board_type = classify_board(all_results, &competitive_ns);

    BoardContext {
        field_contract,
        board_type,
        field_trick_averages,
        competitive_ns,
        competitive_ew,
        ns_typically_declares,
        ew_typically_declares,
        field_declaring_direction,
    }
}

/// Analyze one direction's result on a board using pre-computed field context.
///
/// This is the shared core that both per-player and per-board analysis use.
/// The `role` parameter determines how the result is interpreted (Declarer
/// sees trick diffs as play quality, Defender sees them as defense quality).
fn analyze_direction(
    result: &BoardResult,
    direction: PartnershipDirection,
    role: PlayerRole,
    board_ctx: &BoardContext,
    all_results: &[&BoardResult],
    board_data: Option<&BoardData>,
) -> DirectionAnalysis {
    let matchpoint_pct = calculate_matchpoint_pct(result, all_results, direction);

    // Declarer vs field trick difference (from the actual declarer's perspective)
    let declarer_vs_field = board_ctx.declarer_vs_field(result);

    // Check if contract matches field contract
    let matched_field =
        if let (Some(actual), Some(field)) = (&result.contract, &board_ctx.field_contract) {
            actual.level == field.level && strain_key(actual) == strain_key(field)
        } else {
            false
        };

    // Check if same strain as field
    let same_strain =
        if let (Some(actual), Some(field)) = (&result.contract, &board_ctx.field_contract) {
            strain_key(actual) == strain_key(field)
        } else {
            false
        };

    // Cross-direction: the declaring side at this table differs from who
    // typically declares in the field. This means it's a competitive action.
    let field_is_cross_direction = if result.contract.is_some() {
        board_ctx
            .field_declaring_direction
            .is_some_and(|field_dir| result.declaring_direction() != field_dir)
    } else {
        false
    };

    let went_down = result.tricks_relative.is_some_and(|t| t < 0);
    let tricks_made = result.tricks_made();

    // Look up double-dummy expected tricks for this declarer direction and strain
    let dd_tricks = board_data.and_then(|bd| {
        let contract = result.contract.as_ref()?;
        bd.dd_tricks(result.declarer_direction, contract.strain)
    });

    let cause_ctx = CauseContext {
        role,
        matchpoint_pct,
        declarer_vs_field,
        matched_field_contract: matched_field,
        same_strain_as_field: same_strain,
        player_side_typically_declares: board_ctx.player_side_typically_declares(direction),
        field_is_cross_direction,
        went_down,
        dd_tricks,
        tricks_made,
        board_type: &board_ctx.board_type,
        competitive: board_ctx.competitive_info(direction),
        contract: &result.contract,
        field_contract: &board_ctx.field_contract,
    };
    let (cause, notes) = determine_cause_and_notes(&cause_ctx);

    DirectionAnalysis {
        matchpoint_pct,
        role,
        declarer_vs_field,
        matched_field_contract: matched_field,
        same_strain_as_field: same_strain,
        cause,
        notes,
    }
}

// ==================== Cause Analysis ====================

/// Determine the cause and notes for a board result
///
/// When `same_strain_as_field` is false, trick comparisons are not meaningful
/// (e.g., comparing spade tricks vs diamond tricks). In these cases, the
/// contract choice (Auction) is the primary driver of the result.
fn determine_cause_and_notes(ctx: &CauseContext<'_>) -> (ResultCause, String) {
    let is_good_result = ctx.matchpoint_pct >= 55.0;
    let is_bad_result = ctx.matchpoint_pct <= 45.0;

    match ctx.role {
        PlayerRole::Declarer => {
            // On competitive boards: declaring in own strain at a level
            // below where opponents typically compete → Lucky
            if let Some(comp) = ctx.competitive {
                if let Some(c) = ctx.contract {
                    if c.strain == comp.player_strain
                        && bid_rank_of(comp.opp_max_level, comp.opp_strain) > bid_rank(c)
                        && is_good_result
                    {
                        return (ResultCause::Lucky, "opps failed to compete".to_string());
                    }
                }
            }

            // Cross-direction: this side competed and won the auction when
            // the field has the other side declaring. Never compare strains
            // across directions — it's always competitive dynamics.
            if ctx.field_is_cross_direction {
                if is_good_result {
                    return (ResultCause::Good, "competed successfully".to_string());
                } else if is_bad_result {
                    return (ResultCause::Auction, "competed too high".to_string());
                }
                return (ResultCause::Auction, "competed".to_string());
            }

            let auction_note = if !ctx.matched_field_contract {
                format_auction_note(ctx.contract, ctx.field_contract)
            } else {
                String::new()
            };

            if ctx.same_strain_as_field {
                // DD-based analysis: when matched field contract and DD available,
                // double-dummy tricks are the ground truth benchmark
                if ctx.matched_field_contract {
                    if let (Some(dd), Some(made)) = (ctx.dd_tricks, ctx.tricks_made) {
                        let dd_diff = made as i32 - dd as i32;
                        if dd_diff > 0 {
                            return (ResultCause::Lucky, format!("defense slip (DD {})", dd));
                        } else if dd_diff < 0 {
                            return (ResultCause::Play, format!("below DD ({})", dd));
                        } else {
                            return (ResultCause::Good, format!("DD par ({})", dd));
                        }
                    }
                }

                // Same strain as field: trick comparison is meaningful and primary
                if let Some(diff) = ctx.declarer_vs_field {
                    let tricks_diff = diff.round() as i32;
                    if tricks_diff < 0 {
                        let trick_note =
                            format!("{} {} fewer", -tricks_diff, tricks_word(tricks_diff));
                        let note = if !auction_note.is_empty() {
                            format!("{}, also {}", trick_note, auction_note)
                        } else {
                            trick_note
                        };
                        return (ResultCause::Play, note);
                    } else if tricks_diff > 0 {
                        let trick_note = format!("+{} {}", tricks_diff, tricks_word(tricks_diff));
                        let note = if !auction_note.is_empty() {
                            format!("{}, also {}", trick_note, auction_note)
                        } else {
                            trick_note
                        };
                        if is_good_result {
                            return (ResultCause::Good, note);
                        } else {
                            return (ResultCause::Play, note);
                        }
                    } else if ctx.matched_field_contract {
                        // Matched field contract and field tricks exactly
                        if is_good_result {
                            return (ResultCause::Good, String::new());
                        } else if is_bad_result {
                            return (ResultCause::Unlucky, "field avg".to_string());
                        }
                    }
                }
                // Same strain, lower level, went down: Play error — can't blame
                // the auction for not bidding higher when you can't make what you bid.
                if ctx.went_down {
                    if let (Some(actual), Some(field)) = (ctx.contract, ctx.field_contract) {
                        if actual.level < field.level {
                            return (ResultCause::Play, "went down".to_string());
                        }
                    }
                }
            } else {
                // Different strain from field: contract choice is the primary driver.
                // But on competitive boards where the field contract is in the
                // opponent's strain, "X vs Y" isn't meaningful — it's competitive.
                if let Some(comp) = ctx.competitive {
                    if let Some(field) = ctx.field_contract {
                        if field.strain == comp.opp_strain {
                            if is_good_result {
                                return (ResultCause::Good, "competed successfully".to_string());
                            } else if is_bad_result {
                                return (ResultCause::Auction, "competed too high".to_string());
                            }
                            return (ResultCause::Auction, "competed".to_string());
                        }
                    }
                }
                if !auction_note.is_empty() {
                    if is_good_result {
                        return (ResultCause::Good, auction_note);
                    } else {
                        return (ResultCause::Auction, auction_note);
                    }
                }
            }

            // Fall through: no trick data, or tricks matched, or no auction note
            if !ctx.matched_field_contract {
                if is_good_result {
                    return (ResultCause::Good, auction_note);
                } else {
                    return (ResultCause::Auction, auction_note);
                }
            }
        }
        PlayerRole::Dummy => {
            // Competitive boards: partner declaring in own strain below opponent's max
            if let Some(comp) = ctx.competitive {
                if let Some(c) = ctx.contract {
                    if c.strain == comp.player_strain
                        && bid_rank_of(comp.opp_max_level, comp.opp_strain) > bid_rank(c)
                        && is_good_result
                    {
                        return (ResultCause::Lucky, "opps failed to compete".to_string());
                    }
                }
            }

            // Cross-direction: partner competed and won the auction when
            // the field has the other side declaring.
            if ctx.field_is_cross_direction {
                if is_good_result {
                    return (ResultCause::Good, "competed successfully".to_string());
                } else if is_bad_result {
                    return (ResultCause::Auction, "competed too high".to_string());
                }
                return (ResultCause::Auction, "competed".to_string());
            }

            let auction_note = if !ctx.matched_field_contract {
                format_auction_note(ctx.contract, ctx.field_contract)
            } else {
                String::new()
            };

            if ctx.same_strain_as_field {
                // DD-based analysis: when matched field contract and DD available
                if ctx.matched_field_contract {
                    if let (Some(dd), Some(made)) = (ctx.dd_tricks, ctx.tricks_made) {
                        let dd_diff = made as i32 - dd as i32;
                        if dd_diff > 0 {
                            return (ResultCause::Lucky, format!("defense slip (DD {})", dd));
                        } else if dd_diff < 0 {
                            return (ResultCause::Play, format!("pard below DD ({})", dd));
                        } else {
                            return (ResultCause::Good, format!("DD par ({})", dd));
                        }
                    }
                }

                // Same strain: trick comparison is meaningful
                if let Some(diff) = ctx.declarer_vs_field {
                    let tricks_diff = diff.round() as i32;
                    if tricks_diff < 0 {
                        let trick_note =
                            format!("pard {} {}", tricks_diff, tricks_word(tricks_diff));
                        let note = if !auction_note.is_empty() {
                            format!("{}, also {}", trick_note, auction_note)
                        } else {
                            trick_note
                        };
                        return (ResultCause::Play, note);
                    } else if tricks_diff > 0 {
                        let trick_note =
                            format!("pard +{} {}", tricks_diff, tricks_word(tricks_diff));
                        let note = if !auction_note.is_empty() {
                            format!("{}, also {}", trick_note, auction_note)
                        } else {
                            trick_note
                        };
                        if is_good_result {
                            return (ResultCause::Good, note);
                        }
                        return (ResultCause::Play, note);
                    }
                }
                // Same strain, lower level, went down: Play error
                if ctx.went_down {
                    if let (Some(actual), Some(field)) = (ctx.contract, ctx.field_contract) {
                        if actual.level < field.level {
                            return (ResultCause::Play, "pard went down".to_string());
                        }
                    }
                }
            } else {
                // Different strain: competitive board with field in opponent's strain
                if let Some(comp) = ctx.competitive {
                    if let Some(field) = ctx.field_contract {
                        if field.strain == comp.opp_strain {
                            if is_good_result {
                                return (ResultCause::Good, "competed successfully".to_string());
                            } else if is_bad_result {
                                return (ResultCause::Auction, "competed too high".to_string());
                            }
                            return (ResultCause::Auction, "competed".to_string());
                        }
                    }
                }
                if !auction_note.is_empty() {
                    if is_good_result {
                        return (ResultCause::Good, auction_note);
                    }
                    return (ResultCause::Auction, auction_note);
                }
            }

            // Fall through to auction check
            if !ctx.matched_field_contract {
                if is_good_result {
                    return (ResultCause::Good, auction_note);
                }
                return (ResultCause::Auction, auction_note);
            }
        }
        PlayerRole::Defender => {
            // Competitive boards: defending opponent's strain
            if let Some(comp) = ctx.competitive {
                if let Some(c) = ctx.contract {
                    if c.strain == comp.opp_strain
                        && bid_rank_of(comp.player_max_level, comp.player_strain) > bid_rank(c)
                    {
                        // Player's side outbids this at other tables.
                        // Show the minimum level needed to outbid (not the max reached).
                        let outbid_level = min_outbid_level(c, comp.player_strain)
                            .unwrap_or(comp.player_max_level);
                        let note = format!(
                            "failed to compete to {}{}",
                            outbid_level,
                            strain_display(comp.player_strain)
                        );
                        if is_bad_result {
                            return (ResultCause::Auction, note);
                        } else if is_good_result {
                            return (ResultCause::Lucky, "opps stopped low".to_string());
                        }
                    }
                }
            }

            // Cross-direction: opponents competed and won the auction when
            // the field has our side declaring.
            if ctx.field_is_cross_direction {
                if is_good_result {
                    return (ResultCause::Lucky, "opps competed too high".to_string());
                } else if is_bad_result {
                    return (ResultCause::Auction, "failed to compete".to_string());
                }
                return (ResultCause::Lucky, "opps competed".to_string());
            }

            // Non-competitive defense analysis
            if ctx.same_strain_as_field {
                // DD-based analysis: when matched field contract and DD available
                if ctx.matched_field_contract {
                    if let (Some(dd), Some(made)) = (ctx.dd_tricks, ctx.tricks_made) {
                        let dd_diff = made as i32 - dd as i32;
                        if dd_diff > 0 {
                            // Declarer made more than DD — defense error
                            return (
                                ResultCause::Defense,
                                format!("DD slip (gave {} extra)", dd_diff),
                            );
                        } else if dd_diff < 0 {
                            // Declarer made fewer than DD — good defense or declarer error
                            return (ResultCause::Lucky, format!("held below DD ({})", dd));
                        } else {
                            return (ResultCause::Good, format!("DD par ({})", dd));
                        }
                    }
                }

                // Same strain: trick comparison is meaningful
                if let Some(diff) = ctx.declarer_vs_field {
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
                // Same strain, different level — opponent's auction choice
                if !ctx.matched_field_contract {
                    if let (Some(actual), Some(field)) = (ctx.contract, ctx.field_contract) {
                        if actual.level < field.level {
                            if is_good_result {
                                return (ResultCause::Lucky, "opps underbid".to_string());
                            } else if is_bad_result {
                                return (ResultCause::Unlucky, "opps underbid".to_string());
                            }
                        } else if actual.level > field.level {
                            if is_good_result {
                                return (ResultCause::Lucky, "opps overbid".to_string());
                            } else if is_bad_result {
                                return (ResultCause::Unlucky, "opps overbid".to_string());
                            }
                        }
                    }
                }
            } else if let (Some(actual), Some(field)) = (ctx.contract, ctx.field_contract) {
                // Different strain from field
                if ctx.player_side_typically_declares && bid_rank(actual) < bid_rank(field) {
                    let note = format!("failed to compete to {}", field.display());
                    if is_bad_result {
                        return (ResultCause::Auction, note);
                    } else if is_good_result {
                        return (ResultCause::Good, note);
                    }
                }
                if !ctx.player_side_typically_declares {
                    // Opponents typically declare — different contract is just
                    // which opponent contract we face (e.g., 3NT vs field 5C)
                    if is_bad_result {
                        return (ResultCause::Unlucky, "opps superior contract".to_string());
                    } else if is_good_result {
                        return (ResultCause::Lucky, "opps inferior contract".to_string());
                    }
                }
            }
            // Defender result with no meaningful play/auction signal
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

// ==================== Matchpoint Calculation ====================

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

// ==================== Utilities ====================

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

/// Bidding rank from level and strain (for comparing whether one bid outranks another)
///
/// In bridge, bids rank: 1C < 1D < 1H < 1S < 1NT < 2C < ... < 7NT.
fn bid_rank_of(level: u8, strain: bridge_parsers::Strain) -> u8 {
    use bridge_parsers::Strain;
    let strain_rank = match strain {
        Strain::Clubs => 0,
        Strain::Diamonds => 1,
        Strain::Hearts => 2,
        Strain::Spades => 3,
        Strain::NoTrump => 4,
    };
    (level - 1) * 5 + strain_rank
}

/// Bidding rank of a parsed contract
fn bid_rank(contract: &ParsedContract) -> u8 {
    bid_rank_of(contract.level, contract.strain)
}

/// Display a strain as a short string (C, D, H, S, NT)
fn strain_display(strain: bridge_parsers::Strain) -> &'static str {
    use bridge_parsers::Strain;
    match strain {
        Strain::Clubs => "C",
        Strain::Diamonds => "D",
        Strain::Hearts => "H",
        Strain::Spades => "S",
        Strain::NoTrump => "NT",
    }
}

/// Detect competitive boards and compute context from the player's perspective.
///
/// A board is competitive when both NS and EW have a clear primary strain
/// (each with at least 2 tables declaring in it) and those strains differ.
fn compute_competitive_info(
    board_results: &[&BoardResult],
    player_direction: PartnershipDirection,
) -> Option<CompetitiveInfo> {
    // Use strain key strings as HashMap keys (Strain doesn't impl Hash)
    let mut ns_strains: HashMap<&'static str, (bridge_parsers::Strain, usize, u8)> = HashMap::new();
    let mut ew_strains: HashMap<&'static str, (bridge_parsers::Strain, usize, u8)> = HashMap::new();

    for result in board_results {
        if let Some(contract) = &result.contract {
            let strain = contract.strain;
            let key = strain_display(strain);
            let level = contract.level;
            let map = match result.declaring_direction() {
                PartnershipDirection::NorthSouth => &mut ns_strains,
                PartnershipDirection::EastWest => &mut ew_strains,
            };
            let entry = map.entry(key).or_insert((strain, 0, 0));
            entry.1 += 1;
            entry.2 = entry.2.max(level);
        }
    }

    // Primary strain for each side: most common with at least 2 occurrences
    let ns_primary = ns_strains
        .into_values()
        .filter(|(_, count, _)| *count >= 2)
        .max_by_key(|(_, count, _)| *count)
        .map(|(strain, _, max_level)| (strain, max_level));

    let ew_primary = ew_strains
        .into_values()
        .filter(|(_, count, _)| *count >= 2)
        .max_by_key(|(_, count, _)| *count)
        .map(|(strain, _, max_level)| (strain, max_level));

    match (ns_primary, ew_primary) {
        (Some((ns_s, ns_max)), Some((ew_s, ew_max))) if ns_s != ew_s => {
            let (ps, pm, os, om) = match player_direction {
                PartnershipDirection::NorthSouth => (ns_s, ns_max, ew_s, ew_max),
                PartnershipDirection::EastWest => (ew_s, ew_max, ns_s, ns_max),
            };
            Some(CompetitiveInfo {
                player_strain: ps,
                player_max_level: pm,
                opp_strain: os,
                opp_max_level: om,
            })
        }
        _ => None,
    }
}

/// Whether a contract is at game level or higher for its strain.
fn is_game_level(contract: &ParsedContract) -> bool {
    use bridge_parsers::Strain;
    match contract.strain {
        Strain::Clubs | Strain::Diamonds => contract.level >= 5,
        Strain::Hearts | Strain::Spades => contract.level >= 4,
        Strain::NoTrump => contract.level >= 3,
    }
}

/// Classify a board's primary structural characteristic.
///
/// Priority: Competitive > Slam vs Game > Game vs Partscore > Strain Choice > Flat.
/// Each classification requires at least 2 tables on each side of a split.
fn classify_board(
    all_results: &[&BoardResult],
    competitive_ns: &Option<CompetitiveInfo>,
) -> BoardType {
    // 1. Competitive (already detected)
    if let Some(comp) = competitive_ns {
        return BoardType::Competitive {
            ns_strain: strain_display(comp.player_strain),
            ew_strain: strain_display(comp.opp_strain),
        };
    }

    // Gather per-strain counts for level-based classifications
    let mut by_strain: HashMap<&'static str, (usize, usize, usize)> = HashMap::new();
    // (slam_count, game_count, partscore_count)
    for result in all_results {
        if let Some(contract) = &result.contract {
            let entry = by_strain.entry(strain_key(contract)).or_insert((0, 0, 0));
            if contract.level >= 6 {
                entry.0 += 1;
            } else if is_game_level(contract) {
                entry.1 += 1;
            } else {
                entry.2 += 1;
            }
        }
    }

    // 2. Slam vs Game: strain with 2+ slam and 2+ non-slam
    if let Some((&strain, _)) = by_strain
        .iter()
        .filter(|(_, (slam, game, ps))| *slam >= 2 && (*game + *ps) >= 2)
        .max_by_key(|(_, (slam, _, _))| *slam)
    {
        return BoardType::SlamVsGame { strain };
    }

    // 3. Game vs Partscore: strain with 2+ game and 2+ partscore (excluding slams)
    if let Some((&strain, _)) = by_strain
        .iter()
        .filter(|(_, (_, game, ps))| *game >= 2 && *ps >= 2)
        .max_by_key(|(_, (_, game, _))| *game)
    {
        return BoardType::GameVsPartscore { strain };
    }

    // 4. Strain Choice: same declaring direction, 2+ strains each with 2+ tables
    let mut ns_strains: HashMap<&'static str, usize> = HashMap::new();
    let mut ew_strains: HashMap<&'static str, usize> = HashMap::new();
    for result in all_results {
        if let Some(contract) = &result.contract {
            let map = match result.declaring_direction() {
                PartnershipDirection::NorthSouth => &mut ns_strains,
                PartnershipDirection::EastWest => &mut ew_strains,
            };
            *map.entry(strain_key(contract)).or_insert(0) += 1;
        }
    }

    for strain_counts in [&ns_strains, &ew_strains] {
        let mut qualifying: Vec<_> = strain_counts
            .iter()
            .filter(|(_, &count)| count >= 2)
            .collect();
        if qualifying.len() >= 2 {
            qualifying.sort_by(|a, b| b.1.cmp(a.1));
            return BoardType::StrainChoice {
                strain_a: qualifying[0].0,
                strain_b: qualifying[1].0,
            };
        }
    }

    // 5. Flat
    BoardType::Flat
}

/// Compute the minimum level in a given strain that outbids a contract.
///
/// Returns the smallest level (1–7) such that bidding at that level in `strain`
/// outranks the given contract. Returns `None` if no legal level suffices.
fn min_outbid_level(contract: &ParsedContract, strain: bridge_parsers::Strain) -> Option<u8> {
    let target_rank = bid_rank(contract);
    (1..=7).find(|&level| bid_rank_of(level, strain) > target_rank)
}

/// Pluralize "trick" based on count
fn tricks_word(n: i32) -> &'static str {
    if n.abs() == 1 {
        "trick"
    } else {
        "tricks"
    }
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
