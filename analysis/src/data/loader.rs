//! Public BWS+PBN loader.
//!
//! Thin wrapper that drives the pbn_bws adapter (input format → schema)
//! through the builder (schema → SessionData). Returns the first
//! session's GameData since BWS+PBN is always single-session.

use crate::data::adapters::pbn_bws;
use crate::data::builder::build_sessions;
use crate::data::types::GameData;
use crate::error::{AnalysisError, Result};
use std::collections::HashMap;
use std::path::Path;

/// Load complete game data from BWS + (optional) PBN files.
///
/// `_masterpoints_url` is reserved for a future fetcher; currently ignored.
pub fn load_game_data(
    bws_path: &Path,
    pbn_path: Option<&Path>,
    _masterpoints_url: Option<&str>,
) -> Result<GameData> {
    load_game_data_with_overrides(bws_path, pbn_path, None)
}

/// Same as `load_game_data` but accepts a name-override map (ACBL number →
/// real name) used to replace placeholder seats from BWS files that lack
/// the ACBL name database.
pub fn load_game_data_with_overrides(
    bws_path: &Path,
    pbn_path: Option<&Path>,
    overrides: Option<&HashMap<String, String>>,
) -> Result<GameData> {
    let normalized = pbn_bws::load_normalized(bws_path, pbn_path, overrides)?;
    let mut sessions = build_sessions(&normalized, None)?;
    sessions
        .pop()
        .map(|s| s.data)
        .ok_or_else(|| AnalysisError::MissingData("no sessions in BWS/PBN data".into()))
}
