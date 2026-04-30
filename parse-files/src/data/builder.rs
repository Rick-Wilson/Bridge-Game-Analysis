//! Schema-walking enrich passes called at upload time, plus helpers shared
//! with the BWS/PBN adapter.
//!
//! Pre-2026-04 this module also held `build_sessions` / `build_session_data`
//! / `SessionData`, which converted the schema into a richly-typed `GameData`
//! for the (now-retired) Rust analyzer. With the analyzer gone, the only
//! remaining role is to massage the schema before it gets persisted:
//!
//!   - [`enrich_tricks`] — derive missing `tricks` fields from `score`
//!     under standard scoring, so downstream consumers (the JS analyzer)
//!     don't have to re-derive them per-result.
//!   - [`enrich_handviewer_urls`] — replace adapter-supplied `handviewer_url`
//!     fields with canonical BBO LIN URLs that include a constructed
//!     auction (passes-to-declarer, contract bid, closing passes / X / XX).
//!     Adapter-emitted URLs (especially ACBL Live's "play this hand" link
//!     lifted by the extension) usually only encode the deal.

use crate::data::schema::{Hand as SchemaHand, NormalizedGame};
use crate::data::types::{BoardData, ContractResult, ParsedContract, SeatPlayers};
use bridge_parsers::{Card, Contract, Deal, Direction, Hand, Rank, Suit, Vulnerability};

/// Derive missing `tricks` fields from `score` under standard scoring. A
/// result keeps any tricks value it already has; for the rest we search
/// the legal range (-13..=+7 relative to contract) for a value that
/// produces the recorded score, and fill it in when found. Skips passed-
/// out boards and rows where contract / declarer / score are missing.
pub fn enrich_tricks(game: &mut NormalizedGame) {
    for tournament in &mut game.tournaments {
        for event in &mut tournament.events {
            for session in &mut event.sessions {
                for board in &mut session.boards {
                    let vulnerability = Vulnerability::from_pbn(&board.vulnerability)
                        .unwrap_or(Vulnerability::None);
                    for result in &mut board.results {
                        if result.tricks.is_some() {
                            continue;
                        }
                        let contract = match result
                            .contract
                            .as_deref()
                            .filter(|c| !c.is_empty() && *c != "PASS")
                            .and_then(ParsedContract::parse)
                        {
                            Some(c) => c,
                            None => continue,
                        };
                        let declarer = match result.declarer.as_deref().and_then(parse_direction) {
                            Some(d) => d,
                            None => continue,
                        };
                        let score = match result.score {
                            Some(s) => s,
                            None => continue,
                        };
                        let vulnerable = vulnerability.is_vulnerable(declarer);
                        if let Some(rel) =
                            derive_tricks_relative(&contract, declarer, score, vulnerable)
                        {
                            let tricks = contract.level as i32 + 6 + rel;
                            if (0..=13).contains(&tricks) {
                                result.tricks = Some(tricks as u8);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Walk every result in a NormalizedGame and replace `handviewer_url` with
/// a canonical BBO LIN URL built from the deal, contract, declarer, and
/// player names. The constructed URL includes an auction (passes from
/// dealer to declarer, the contract bid, then closing passes / X / XX)
/// so the BBO viewer renders the bidding sequence.
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

// ---- shared helpers (also used by the BWS/PBN adapter via `pub`) ----

/// Convert the schema's deal representation to bridge_parsers' Deal.
pub(crate) fn convert_deal(deal: &crate::data::schema::Deal) -> Option<Deal> {
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

pub(crate) fn parse_direction(s: &str) -> Option<Direction> {
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
/// of each whitespace-separated word. The BWS adapter uses this so the LIN
/// URL it builds matches what `display_name` (in the SPA's JS port) produces.
pub(crate) fn title_case(name: &str) -> String {
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
