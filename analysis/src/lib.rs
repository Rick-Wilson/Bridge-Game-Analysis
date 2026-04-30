pub mod config;
pub mod data;
pub mod error;
pub mod identity;

// The analysis layer (matchpoint / DVF / cause-analysis) used to live here
// too in `pub mod metrics`; it has been retired in favor of the JS port
// shipped in the SPA. The remaining surface is schema definitions, the
// BWS/PBN adapter, and a couple of schema-walk enrich-passes (tricks +
// handviewer-url canonicalization) called at upload time. The data-layer
// types that survived (GameData, SessionData, etc.) continue to back the
// upload-response shaping; a follow-up commit shrinks them too.
pub use config::Config;
pub use data::{
    build_sessions, enrich_handviewer_urls, enrich_tricks, load_game_data,
    load_game_data_with_overrides, parse_normalized, render_par_display, BoardData, BoardResult,
    ContractResult, GameData, NormalizedGame, ParContract, ParsedContract, SchemaParseError,
    SeatPlayers, SessionData,
};
pub use error::{AnalysisError, Result};
pub use identity::{normalize_name, Partnership, PartnershipDirection, Player, PlayerId};

// Re-export useful types from bridge-parsers
pub use bridge_parsers::{Direction, Doubled, Strain, Vulnerability};
