//! Build `GameData` instances from a normalized JSON document.
//!
//! This is the single common path that downstream analysis flows through.
//! Adapters (PBN/BWS, ACBL Live extension, future sources) all produce a
//! `NormalizedGame`; the builder turns that into one `GameData` per
//! bridge session contained in the document.

use crate::data::schema::{
    self, Hand as SchemaHand, NormalizedGame, Pair as SchemaPair, Session as SchemaSession,
};
use crate::data::types::{
    BoardData, BoardResult, ContractResult, DdStrains, DoubleDummyTricks, GameData, ParContract,
    ParsedContract, SeatPlayers,
};
use crate::error::{AnalysisError, Result};
use crate::identity::{Partnership, PlayerId};
use bridge_parsers::{Card, Contract, Deal, Direction, Hand, Rank, Suit, Vulnerability};
use std::collections::HashMap;

/// One bridge session worth of data with a display label.
///
/// A NormalizedGame can carry many sessions (one tournament with multiple
/// events, an event with morning + afternoon sessions, or a player-history
/// document). The builder produces one `SessionData` per session, with
/// `session_idx` matching the flattened position in the document.
#[derive(Debug)]
pub struct SessionData {
    /// Human-readable label combining tournament/event/date/session_number.
    pub label: String,
    /// Position in the flattened session list (0-based).
    pub session_idx: u32,
    /// All boards/results for this session.
    pub data: GameData,
}

/// Build session data from a parsed normalized JSON document.
///
/// `overrides` is an optional map of ACBL number → display name. When a
/// player in the document has an ACBL number that matches and the name
/// is a "Player N-3"-style placeholder, the override is applied at
/// PlayerId-creation time so the canonical name is right downstream.
pub fn build_sessions(
    game: &NormalizedGame,
    overrides: Option<&HashMap<String, String>>,
) -> Result<Vec<SessionData>> {
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
                let data = build_session_data(
                    session,
                    event.name.as_deref().or(tournament.name.as_deref()),
                    event.date.as_deref(),
                    overrides,
                )?;
                out.push(SessionData {
                    label,
                    session_idx,
                    data,
                });
                session_idx += 1;
            }
        }
    }
    Ok(out)
}

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

fn build_session_data(
    session: &SchemaSession,
    event_name: Option<&str>,
    event_date: Option<&str>,
    overrides: Option<&HashMap<String, String>>,
) -> Result<GameData> {
    let mut data = GameData::new();
    data.event_name = event_name.map(|s| s.to_string());
    data.event_date = event_date.map(|s| s.to_string());

    // Boards first — every result needs to look up board context for vulnerability.
    for board in &session.boards {
        let board_data = convert_board(board)?;
        data.boards.insert(board.number, board_data);
    }

    // Walk every result and register players + build BoardResult entries.
    for board in &session.boards {
        for result in &board.results {
            let board_number = board.number;
            // Capture vulnerability for declarer-vs-vul math; release the
            // immutable borrow before mutating the registry below.
            let board_vul = data.boards.get(&board_number).map(|b| b.vulnerability);

            // Resolve players. ns_pair.players[0] = North, [1] = South;
            // ew_pair.players[0] = West, [1] = East. Adapters must follow
            // this convention when emitting the schema.
            let n = resolve_player(&result.ns_pair, 0, &mut data, overrides);
            let s = resolve_player(&result.ns_pair, 1, &mut data, overrides);
            let w = resolve_player(&result.ew_pair, 0, &mut data, overrides);
            let e = resolve_player(&result.ew_pair, 1, &mut data, overrides);

            let (n_id, s_id, e_id, w_id) = match (n, s, e, w) {
                (Some(n), Some(s), Some(e), Some(w)) => (n, s, e, w),
                _ => continue, // Skip rows missing a seat
            };

            // Pair-number lookup for the names-overlay flow. Section is "A" by
            // default since the schema sometimes omits it; we map to (1, num).
            if let Some(num) = result.ns_pair.number {
                data.pairs_by_number
                    .entry((section_key(result.ns_pair.section.as_deref()), num))
                    .or_insert_with(|| (n_id.clone(), s_id.clone()));
            }
            if let Some(num) = result.ew_pair.number {
                data.pairs_by_number
                    .entry((section_key(result.ew_pair.section.as_deref()), num))
                    .or_insert_with(|| (w_id.clone(), e_id.clone()));
            }

            let ns_pair = Partnership::new_seated(n_id.clone(), s_id.clone(), &n_id);
            let ew_pair = Partnership::new_seated(e_id.clone(), w_id.clone(), &w_id);

            // Treat null contract or "PASS" as a passed-out board.
            let contract_str = result.contract.as_deref().map(|c| c.trim());
            let is_pass = matches!(contract_str, Some("PASS") | Some("Passed Out") | Some(""));
            let contract: Option<ParsedContract> = match (contract_str, is_pass) {
                (None, _) | (_, true) => None,
                (Some(s), false) => ParsedContract::parse(s),
            };

            let declarer_dir = result
                .declarer
                .as_deref()
                .and_then(parse_direction)
                .unwrap_or(Direction::North);

            let declarer_id = match declarer_dir {
                Direction::North => n_id,
                Direction::South => s_id,
                Direction::East => e_id,
                Direction::West => w_id,
            };

            // Tricks: prefer schema's absolute `tricks` field; fall back to
            // deriving from score+contract+vulnerability when contract is known.
            let tricks_relative = match (&contract, result.tricks) {
                (Some(c), Some(t)) => Some(t as i32 - c.level as i32 - 6),
                (Some(c), None) => result.score.and_then(|s| {
                    let vulnerable = board_vul
                        .map(|v| v.is_vulnerable(declarer_dir))
                        .unwrap_or(false);
                    derive_tricks_relative(c, declarer_dir, s, vulnerable)
                }),
                _ => None,
            };

            let ns_score = result.score.unwrap_or(0);

            let board_result = BoardResult {
                board_number,
                ns_pair,
                ew_pair,
                declarer_direction: declarer_dir,
                declarer: declarer_id,
                contract,
                tricks_relative,
                ns_score,
            };
            data.results.push(board_result);
        }
    }

    Ok(data)
}

/// Convert a schema Board to a typed BoardData. Errors only on malformed
/// dealer/vulnerability strings (every other field can be permissively absent).
fn convert_board(board: &schema::Board) -> Result<BoardData> {
    let dealer = parse_direction(&board.dealer)
        .ok_or_else(|| AnalysisError::InvalidData(format!("invalid dealer: {:?}", board.dealer)))?;
    let vulnerability = Vulnerability::from_pbn(&board.vulnerability).ok_or_else(|| {
        AnalysisError::InvalidData(format!("invalid vulnerability: {:?}", board.vulnerability))
    })?;

    let deal = board.deal.as_ref().and_then(convert_deal);
    let double_dummy = board.double_dummy.as_ref().map(convert_dd);
    let par = convert_par(&board.par, &vulnerability);

    Ok(BoardData {
        number: board.number,
        dealer,
        vulnerability,
        deal,
        double_dummy,
        par,
    })
}

fn convert_deal(deal: &schema::Deal) -> Option<Deal> {
    let mut out = Deal::new();
    out.set_hand(Direction::North, hand_from_schema(&deal.north)?);
    out.set_hand(Direction::East, hand_from_schema(&deal.east)?);
    out.set_hand(Direction::South, hand_from_schema(&deal.south)?);
    out.set_hand(Direction::West, hand_from_schema(&deal.west)?);
    if !out.has_cards() {
        return None;
    }
    Some(out)
}

fn hand_from_schema(hand: &SchemaHand) -> Option<Hand> {
    let mut out = Hand::new();
    for (suit, ranks) in [
        (Suit::Spades, &hand.spades),
        (Suit::Hearts, &hand.hearts),
        (Suit::Diamonds, &hand.diamonds),
        (Suit::Clubs, &hand.clubs),
    ] {
        for rank_str in ranks {
            let rank = parse_rank(rank_str)?;
            out.add_card(Card::new(suit, rank));
        }
    }
    Some(out)
}

/// Parse a schema rank string. Schema uses "A","K","Q","J","10","9".."2".
fn parse_rank(s: &str) -> Option<Rank> {
    if s == "10" {
        Rank::from_char('T')
    } else if s.len() == 1 {
        Rank::from_char(s.chars().next()?)
    } else {
        None
    }
}

fn convert_dd(dd: &schema::DoubleDummy) -> DoubleDummyTricks {
    let mut out = DoubleDummyTricks::new();
    if let Some(s) = &dd.north {
        out.insert(Direction::North, dd_strains_from_schema(s));
    }
    if let Some(s) = &dd.east {
        out.insert(Direction::East, dd_strains_from_schema(s));
    }
    if let Some(s) = &dd.south {
        out.insert(Direction::South, dd_strains_from_schema(s));
    }
    if let Some(s) = &dd.west {
        out.insert(Direction::West, dd_strains_from_schema(s));
    }
    out
}

fn dd_strains_from_schema(s: &schema::DdStrains) -> DdStrains {
    DdStrains {
        clubs: s.clubs,
        diamonds: s.diamonds,
        hearts: s.hearts,
        spades: s.spades,
        no_trump: s.no_trump,
    }
}

fn convert_par(par: &[schema::Par], vulnerability: &Vulnerability) -> Vec<ParContract> {
    par.iter()
        .filter_map(|p| {
            let contract = ParsedContract::parse(&p.contract)?;
            let declarer = parse_direction(&p.declarer)?;
            let vulnerable = vulnerability.is_vulnerable(declarer);
            let tricks_relative = derive_tricks_relative(&contract, declarer, p.score, vulnerable);
            Some(ParContract {
                score: p.score,
                contract,
                declarer,
                tricks_relative,
            })
        })
        .collect()
}

/// Resolve a player at a seat within a Pair, registering with the registry
/// (and applying name overrides for placeholder names).
fn resolve_player(
    pair: &SchemaPair,
    idx: usize,
    data: &mut GameData,
    overrides: Option<&HashMap<String, String>>,
) -> Option<PlayerId> {
    let p = pair.players.get(idx)?;
    let mut name = p.name.clone();
    if name.starts_with("Player ") {
        if let (Some(overrides), Some(acbl)) = (overrides, p.acbl_id.as_ref()) {
            if let Some(real) = overrides.get(acbl) {
                name = real.clone();
            }
        }
    }
    Some(data.players.get_or_create(&name, p.acbl_id.clone()))
}

/// Default section number for the pairs-by-number lookup. The legacy BWS
/// path keys by integer section codes; the schema expresses sections as
/// strings, so we map "A"→1, "B"→2, ..., and missing → 1.
fn section_key(section: Option<&str>) -> i32 {
    match section.map(|s| s.trim().to_uppercase()) {
        Some(s) if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch.is_ascii_uppercase() {
                (ch as i32) - ('A' as i32) + 1
            } else {
                1
            }
        }
        _ => 1,
    }
}

fn parse_direction(s: &str) -> Option<Direction> {
    match s.trim().to_uppercase().as_str() {
        "N" | "NORTH" => Some(Direction::North),
        "E" | "EAST" => Some(Direction::East),
        "S" | "SOUTH" => Some(Direction::South),
        "W" | "WEST" => Some(Direction::West),
        _ => None,
    }
}

/// Given (contract, declarer, NS-perspective score, vulnerable), find the
/// trick-relative number that produces this score under standard scoring.
/// Returns None if no value in the legal range matches.
fn derive_tricks_relative(
    contract: &ParsedContract,
    declarer: Direction,
    ns_score: i32,
    vulnerable: bool,
) -> Option<i32> {
    let raw_target = match declarer {
        Direction::North | Direction::South => ns_score,
        Direction::East | Direction::West => -ns_score,
    };
    let bp = Contract::new(
        contract.level,
        contract.strain,
        contract.doubled,
        direction_to_char(declarer),
    );
    // Search the legal range: down 13 (all defenders' tricks) up to +6 (max overtricks).
    (-13i32..=7).find(|&rel| bp.score(rel, vulnerable) == raw_target)
}

fn direction_to_char(d: Direction) -> char {
    match d {
        Direction::North => 'N',
        Direction::East => 'E',
        Direction::South => 'S',
        Direction::West => 'W',
    }
}

/// Title-case a name: lowercase everything, then capitalize the first letter
/// of each whitespace-separated word. Mirrors PlayerId::display_name() so the
/// LIN URL we build matches what /api/board's response builder produces.
fn title_case(name: &str) -> String {
    name.trim()
        .to_lowercase()
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

/// Walk every result in a NormalizedGame and replace `handviewer_url` with
/// a canonical BBO LIN URL built from the deal, contract, declarer, and
/// player names. The canonical URL includes a constructed auction
/// (passes-from-dealer-to-declarer, the contract bid, then closing
/// passes/X/XX) so the BBO viewer renders the bidding sequence — adapter-
/// supplied URLs (e.g. ACBL Live's "play this hand" link, lifted by the
/// extension) often only encode the deal and let BBO render auto-computed
/// par/DD analysis instead.
///
/// Called by /api/upload-normalized before persisting data.json so every
/// request that reads the schema (engine=js, /api/normalized streaming,
/// /api/board response builder) sees the canonical URL.
pub fn enrich_handviewer_urls(game: &mut NormalizedGame) {
    for tournament in &mut game.tournaments {
        for event in &mut tournament.events {
            for session in &mut event.sessions {
                for board in &mut session.boards {
                    let dealer = match parse_direction(&board.dealer) {
                        Some(d) => d,
                        None => continue,
                    };
                    let vulnerability = Vulnerability::from_pbn(&board.vulnerability)
                        .unwrap_or(Vulnerability::None);
                    let deal = match board.deal.as_ref().and_then(convert_deal) {
                        Some(d) => d,
                        None => continue,
                    };
                    let bd = BoardData {
                        number: board.number,
                        dealer,
                        vulnerability,
                        deal: Some(deal),
                        double_dummy: None,
                        par: Vec::new(),
                    };
                    for result in &mut board.results {
                        let n = result.ns_pair.players.first().map(|p| title_case(&p.name));
                        let s = result.ns_pair.players.get(1).map(|p| title_case(&p.name));
                        let w = result.ew_pair.players.first().map(|p| title_case(&p.name));
                        let e = result.ew_pair.players.get(1).map(|p| title_case(&p.name));
                        let players = match (n, s, e, w) {
                            (Some(n), Some(s), Some(e), Some(w)) => Some(SeatPlayers {
                                north: n,
                                south: s,
                                east: e,
                                west: w,
                            }),
                            _ => None,
                        };
                        let contract_result = match (
                            result
                                .contract
                                .as_deref()
                                .filter(|c| !c.is_empty() && *c != "PASS")
                                .and_then(ParsedContract::parse),
                            result.declarer.as_deref().and_then(parse_direction),
                        ) {
                            (Some(c), Some(d)) => Some(ContractResult {
                                contract: c,
                                declarer: d,
                            }),
                            _ => None,
                        };
                        if let Some(url) =
                            bd.bbo_handviewer_url(players.as_ref(), contract_result.as_ref())
                        {
                            result.handviewer_url = Some(url);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::schema::parse_normalized;

    /// A two-session document produces two SessionData entries with
    /// distinct labels and independent GameData.
    #[test]
    fn build_two_sessions() {
        let json = r#"{
            "schema_version": "1.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": [{
                "name": "Demo Sectional",
                "events": [{
                    "name": "Open Pairs",
                    "date": "2026-04-25",
                    "sessions": [
                        {
                            "session_number": 1,
                            "boards": [{
                                "number": 1,
                                "dealer": "N",
                                "vulnerability": "None",
                                "results": [{
                                    "contract": "3NT",
                                    "declarer": "N",
                                    "tricks": 9,
                                    "score": 400,
                                    "ns_pair": {"players": [{"name": "Alice"}, {"name": "Bob"}]},
                                    "ew_pair": {"players": [{"name": "Carol"}, {"name": "Dave"}]}
                                }]
                            }]
                        },
                        {
                            "session_number": 2,
                            "boards": [{
                                "number": 1,
                                "dealer": "E",
                                "vulnerability": "NS",
                                "results": [{
                                    "contract": "4S",
                                    "declarer": "S",
                                    "tricks": 10,
                                    "score": 620,
                                    "ns_pair": {"players": [{"name": "Eve"}, {"name": "Frank"}]},
                                    "ew_pair": {"players": [{"name": "Grace"}, {"name": "Heidi"}]}
                                }]
                            }]
                        }
                    ]
                }]
            }]
        }"#;
        let game = parse_normalized(json).expect("parse");
        let sessions = build_sessions(&game, None).expect("build");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_idx, 0);
        assert_eq!(sessions[1].session_idx, 1);
        assert!(sessions[0].label.contains("Session 1"));
        assert!(sessions[1].label.contains("Session 2"));
        // Each session has its own players (no cross-contamination).
        assert_eq!(sessions[0].data.results.len(), 1);
        assert_eq!(sessions[1].data.results.len(), 1);
        assert_eq!(sessions[0].data.results[0].ns_score, 400);
        assert_eq!(sessions[1].data.results[0].ns_score, 620);
    }

    /// Tricks and contract are both required to compute tricks_relative
    /// directly. When `tricks` is null, we fall back to deriving from the
    /// score (deterministic for non-doubled contracts).
    #[test]
    fn derives_tricks_relative_from_score() {
        let json = r#"{
            "schema_version": "1.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": [{
                "events": [{
                    "sessions": [{
                        "session_number": 1,
                        "boards": [{
                            "number": 1,
                            "dealer": "N",
                            "vulnerability": "None",
                            "results": [{
                                "contract": "3NT",
                                "declarer": "N",
                                "score": 430,
                                "ns_pair": {"players": [{"name": "A"}, {"name": "B"}]},
                                "ew_pair": {"players": [{"name": "C"}, {"name": "D"}]}
                            }]
                        }]
                    }]
                }]
            }]
        }"#;
        let game = parse_normalized(json).expect("parse");
        let sessions = build_sessions(&game, None).expect("build");
        // 3NT non-vul making 10 tricks (= +1) scores 430.
        assert_eq!(sessions[0].data.results[0].tricks_relative, Some(1));
    }

    /// Deal arrays with "10" should round-trip as the Ten card.
    #[test]
    fn parses_ten_card() {
        let hand = SchemaHand {
            spades: vec!["A".into(), "10".into()],
            hearts: vec![],
            diamonds: vec![],
            clubs: vec![],
        };
        let h = hand_from_schema(&hand).expect("parse");
        assert_eq!(h.suit_length(Suit::Spades), 2);
    }

    /// Tied par contracts both flow through and pick up tricks_relative.
    #[test]
    fn tied_par_with_tricks_derived() {
        let json = r#"{
            "schema_version": "1.0",
            "source": "test",
            "fetched_at": "2026-04-26T18:30:00Z",
            "tournaments": [{
                "events": [{
                    "sessions": [{
                        "session_number": 1,
                        "boards": [{
                            "number": 1,
                            "dealer": "N",
                            "vulnerability": "None",
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
        let game = parse_normalized(json).expect("parse");
        let sessions = build_sessions(&game, None).expect("build");
        let board = &sessions[0].data.boards[&1];
        assert_eq!(board.par.len(), 2);
        // 4H non-vul making 10 tricks (= 0) scores 420.
        assert_eq!(board.par[0].tricks_relative, Some(0));
    }
}
