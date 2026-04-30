pub mod data;
pub mod error;

// The analyzer (matchpoint / DVF / cause-analysis) used to live in
// `pub mod metrics`; it has been retired in favor of the JS port shipped
// in the SPA. The crate is now scoped to the BWS/PBN → JSON translation
// layer: schema definitions, the BWS/PBN adapter, and a couple of
// schema-walk enrich-passes (tricks + handviewer-url canonicalization)
// called at upload time. Everything that produced GameData / SessionData
// / PlayerRegistry from the schema is gone — the web crate walks the
// schema directly via web/src/upload_helpers.rs now.
pub use data::{
    enrich_handviewer_urls, enrich_tricks, parse_normalized, NormalizedGame, SchemaParseError,
};
pub use error::{AnalysisError, Result};
