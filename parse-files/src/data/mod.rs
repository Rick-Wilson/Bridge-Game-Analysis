pub mod adapters;
mod builder;
pub mod schema;
mod types;

pub use builder::{enrich_handviewer_urls, enrich_tricks};
pub use schema::{parse_normalized, NormalizedGame, ParseError as SchemaParseError};
pub use types::{BoardData, ContractResult, ParsedContract, SeatPlayers};
