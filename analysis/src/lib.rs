pub mod config;
pub mod data;
pub mod error;
pub mod identity;
pub mod metrics;

// Re-export main types for convenience
pub use config::Config;
pub use data::{
    load_game_data, BoardData, BoardResult, ContractResult, GameData, ParsedContract, SeatPlayers,
};
pub use error::{AnalysisError, Result};
pub use identity::{normalize_name, Partnership, PartnershipDirection, Player, PlayerId};
pub use metrics::{
    analyze_bidding_performance, analyze_board, analyze_declarer_performance, analyze_player,
    BiddingPerformance, BiddingResult, BoardAnalysis, BoardContext, BoardTableResult,
    DeclarerPerformance, DeclarerResult, DirectionAnalysis, PlayerAnalysis, PlayerBoardResult,
    PlayerRole, ResultCause, StrainPerformance,
};

// Re-export useful types from bridge-parsers
pub use bridge_parsers::{Direction, Doubled, Strain, Vulnerability};
