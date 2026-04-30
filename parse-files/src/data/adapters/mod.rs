//! Source-specific input adapters that produce `NormalizedGame` documents.
//!
//! Every adapter reads its native format and emits the same JSON-shaped
//! schema; everything downstream (the builder, then analysis) reads only
//! from that schema and is unaware of input format.

pub mod pbn_bws;

pub use pbn_bws::load_normalized;
