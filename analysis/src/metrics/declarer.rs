use crate::data::{GameData, ParsedContract};
use crate::identity::PlayerId;
use bridge_parsers::{Direction, Strain};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// String key for strain (since Strain doesn't implement Hash)
fn strain_key(strain: Strain) -> &'static str {
    match strain {
        Strain::Clubs => "C",
        Strain::Diamonds => "D",
        Strain::Hearts => "H",
        Strain::Spades => "S",
        Strain::NoTrump => "NT",
    }
}

/// A single declarer play result
#[derive(Debug, Clone)]
pub struct DeclarerResult {
    pub player: PlayerId,
    pub board_number: u32,
    pub direction: Direction,
    pub contract: ParsedContract,
    pub tricks_made: u8,
    pub tricks_relative: i32,
}

/// Performance in a specific strain
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrainPerformance {
    /// Number of boards declared in this strain
    pub boards: u32,
    /// Average tricks vs field (positive = better than field)
    pub avg_tricks_vs_field: f64,
    /// Total tricks differential
    pub total_tricks_differential: f64,
}

/// Aggregated declarer performance for a player
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarerPerformance {
    pub player: PlayerId,
    /// Average tricks over/under field for same strain on same board
    pub avg_tricks_vs_field: f64,
    /// Total boards declared
    pub boards_declared: u32,
    /// Breakdown by strain (keyed by strain abbreviation: C, D, H, S, NT)
    pub by_strain: HashMap<String, StrainPerformance>,
}

impl DeclarerPerformance {
    /// Get performance for a specific strain
    pub fn strain_performance(&self, strain: Strain) -> Option<&StrainPerformance> {
        self.by_strain.get(strain_key(strain))
    }
}

/// Compute field averages for each (board, strain) combination
/// Returns map of (board_number, strain_key) -> (average tricks, count)
fn compute_field_averages(data: &GameData) -> HashMap<(u32, &'static str), (f64, usize)> {
    let mut grouped: HashMap<(u32, &'static str), Vec<u8>> = HashMap::new();

    for result in &data.results {
        if let (Some(contract), Some(tricks)) = (&result.contract, result.tricks_made()) {
            let key = (result.board_number, strain_key(contract.strain));
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

/// Analyze declarer performance for all players
pub fn analyze_declarer_performance(data: &GameData) -> Vec<DeclarerPerformance> {
    // Compute field averages
    let field_averages = compute_field_averages(data);

    // Group results by declarer
    let mut by_declarer: HashMap<PlayerId, Vec<DeclarerResult>> = HashMap::new();

    for result in &data.results {
        if let (Some(contract), Some(tricks_made), Some(tricks_rel)) = (
            &result.contract,
            result.tricks_made(),
            result.tricks_relative,
        ) {
            let declarer_result = DeclarerResult {
                player: result.declarer.clone(),
                board_number: result.board_number,
                direction: result.declarer_direction,
                contract: contract.clone(),
                tricks_made,
                tricks_relative: tricks_rel,
            };
            by_declarer
                .entry(result.declarer.clone())
                .or_default()
                .push(declarer_result);
        }
    }

    // Compute performance for each player
    let mut performances: Vec<DeclarerPerformance> = by_declarer
        .into_iter()
        .map(|(player, results)| compute_declarer_performance(player, results, &field_averages))
        .collect();

    // Sort by average tricks vs field (descending)
    performances.sort_by(|a, b| {
        b.avg_tricks_vs_field
            .partial_cmp(&a.avg_tricks_vs_field)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    performances
}

/// Compute aggregated performance for a single player
fn compute_declarer_performance(
    player: PlayerId,
    results: Vec<DeclarerResult>,
    field_averages: &HashMap<(u32, &'static str), (f64, usize)>,
) -> DeclarerPerformance {
    let mut by_strain: HashMap<&'static str, Vec<f64>> = HashMap::new();
    let mut total_differential = 0.0;
    let mut count = 0;

    for result in &results {
        let sk = strain_key(result.contract.strain);
        let key = (result.board_number, sk);

        if let Some(&(field_avg, field_count)) = field_averages.get(&key) {
            // Only compare if there were multiple declarers in this strain
            if field_count > 1 {
                let differential = result.tricks_made as f64 - field_avg;
                total_differential += differential;
                count += 1;

                by_strain.entry(sk).or_default().push(differential);
            }
        }
    }

    let avg_tricks_vs_field = if count > 0 {
        total_differential / count as f64
    } else {
        0.0
    };

    let by_strain = by_strain
        .into_iter()
        .map(|(strain, diffs)| {
            let boards = diffs.len() as u32;
            let total: f64 = diffs.iter().sum();
            let avg = if boards > 0 {
                total / boards as f64
            } else {
                0.0
            };
            (
                strain.to_string(),
                StrainPerformance {
                    boards,
                    avg_tricks_vs_field: avg,
                    total_tricks_differential: total,
                },
            )
        })
        .collect();

    DeclarerPerformance {
        player,
        avg_tricks_vs_field,
        boards_declared: results.len() as u32,
        by_strain,
    }
}

/// Analyze declarer performance for a specific player
#[allow(dead_code)]
pub fn analyze_player_declarer_performance(
    data: &GameData,
    player_name: &str,
) -> Option<DeclarerPerformance> {
    let normalized = crate::identity::normalize_name(player_name);
    analyze_declarer_performance(data)
        .into_iter()
        .find(|p| p.player.canonical_name == normalized)
}
