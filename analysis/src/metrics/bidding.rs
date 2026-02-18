use crate::data::{GameData, ParsedContract};
use crate::identity::Partnership;
use bridge_parsers::Strain;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bidding accuracy for a partnership on a single board
#[derive(Debug, Clone)]
pub struct BiddingResult {
    pub partnership: Partnership,
    pub board_number: u32,
    /// Did they reach par contract strain?
    pub reached_par_strain: Option<bool>,
    /// Did they reach the most common contract in the field?
    pub reached_field_contract: bool,
    /// The actual contract bid
    pub actual_contract: ParsedContract,
    /// The par contract from double dummy analysis
    pub par_contract: Option<String>,
    /// The most common contract in the field
    pub field_contract: ParsedContract,
}

/// Aggregated bidding performance for a partnership
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiddingPerformance {
    pub partnership: Partnership,
    /// Percentage of boards where par strain was reached
    pub par_strain_accuracy: Option<f64>,
    /// Percentage of boards matching field contract
    pub field_agreement: f64,
    /// Total boards analyzed
    pub boards_analyzed: u32,
    /// Boards with par data available
    pub boards_with_par: u32,
}

/// Compute the most common contract for each board
fn compute_field_contracts(data: &GameData) -> HashMap<u32, ParsedContract> {
    let mut by_board: HashMap<u32, HashMap<String, (ParsedContract, usize)>> = HashMap::new();

    for result in &data.results {
        if let Some(contract) = &result.contract {
            let key = contract.display();
            by_board
                .entry(result.board_number)
                .or_default()
                .entry(key.clone())
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

/// Parse par contract strain from PBN par contract string
/// Par contract strings can be like "NS 4S" or "EW 3NT" or "4S NS"
fn parse_par_strain(par_str: &str) -> Option<Strain> {
    let upper = par_str.to_uppercase();

    // Look for strain indicators
    if upper.contains("NT") || upper.contains("N ") {
        return Some(Strain::NoTrump);
    }

    // Look for suit indicators with a level
    for c in upper.chars() {
        match c {
            'S' => {
                // Make sure it's not part of "NS" by checking context
                if !upper.contains("NS") || upper.contains("S ") || upper.ends_with('S') {
                    // Check if there's a number before it
                    let s_pos = upper.find('S')?;
                    if s_pos > 0 {
                        let prev_char = upper.chars().nth(s_pos - 1)?;
                        if prev_char.is_ascii_digit() {
                            return Some(Strain::Spades);
                        }
                    }
                }
            }
            'H' => return Some(Strain::Hearts),
            'D' => return Some(Strain::Diamonds),
            'C' => return Some(Strain::Clubs),
            _ => {}
        }
    }

    // Try parsing as a contract string
    if let Some(contract) = ParsedContract::parse(&upper) {
        return Some(contract.strain);
    }

    None
}

/// Analyze bidding performance for all partnerships
pub fn analyze_bidding_performance(data: &GameData) -> Vec<BiddingPerformance> {
    let field_contracts = compute_field_contracts(data);

    // Group results by declaring partnership
    let mut by_partnership: HashMap<Partnership, Vec<BiddingResult>> = HashMap::new();

    for result in &data.results {
        if let Some(contract) = &result.contract {
            let partnership = result.declaring_partnership().clone();
            let board_data = data.boards.get(&result.board_number);

            // Get par contract string
            let par_contract = board_data.and_then(|b| b.par_contract.clone());

            // Check if reached par strain
            let reached_par_strain = par_contract.as_ref().and_then(|par| {
                parse_par_strain(par).map(|par_strain| contract.strain == par_strain)
            });

            // Get field contract for this board
            let field_contract = match field_contracts.get(&result.board_number) {
                Some(fc) => fc.clone(),
                None => continue,
            };

            // Check if reached field contract
            let reached_field =
                contract.level == field_contract.level && contract.strain == field_contract.strain;

            let bidding_result = BiddingResult {
                partnership: partnership.clone(),
                board_number: result.board_number,
                reached_par_strain,
                reached_field_contract: reached_field,
                actual_contract: contract.clone(),
                par_contract,
                field_contract,
            };

            by_partnership
                .entry(partnership)
                .or_default()
                .push(bidding_result);
        }
    }

    // Aggregate performance for each partnership
    let mut performances: Vec<BiddingPerformance> = by_partnership
        .into_iter()
        .map(|(partnership, results)| compute_bidding_performance(partnership, results))
        .collect();

    // Sort by field agreement (descending)
    performances.sort_by(|a, b| {
        b.field_agreement
            .partial_cmp(&a.field_agreement)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    performances
}

/// Compute aggregated bidding performance for a partnership
fn compute_bidding_performance(
    partnership: Partnership,
    results: Vec<BiddingResult>,
) -> BiddingPerformance {
    let boards_analyzed = results.len() as u32;

    // Count field matches
    let field_matches = results.iter().filter(|r| r.reached_field_contract).count();
    let field_agreement = if boards_analyzed > 0 {
        (field_matches as f64 / boards_analyzed as f64) * 100.0
    } else {
        0.0
    };

    // Count par strain matches (only for boards with par data)
    let par_results: Vec<_> = results
        .iter()
        .filter_map(|r| r.reached_par_strain)
        .collect();
    let boards_with_par = par_results.len() as u32;
    let par_matches = par_results.iter().filter(|&&x| x).count();
    let par_strain_accuracy = if boards_with_par > 0 {
        Some((par_matches as f64 / boards_with_par as f64) * 100.0)
    } else {
        None
    };

    BiddingPerformance {
        partnership,
        par_strain_accuracy,
        field_agreement,
        boards_analyzed,
        boards_with_par,
    }
}

/// Analyze bidding performance for a specific partnership
#[allow(dead_code)]
pub fn analyze_partnership_bidding(
    data: &GameData,
    player1_name: &str,
    player2_name: &str,
) -> Option<BiddingPerformance> {
    let norm1 = crate::identity::normalize_name(player1_name);
    let norm2 = crate::identity::normalize_name(player2_name);

    analyze_bidding_performance(data).into_iter().find(|p| {
        (p.partnership.player1.canonical_name == norm1
            && p.partnership.player2.canonical_name == norm2)
            || (p.partnership.player1.canonical_name == norm2
                && p.partnership.player2.canonical_name == norm1)
    })
}
