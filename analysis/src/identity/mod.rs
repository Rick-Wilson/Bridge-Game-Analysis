mod player;
mod partnership;

pub use player::{normalize_name, Player, PlayerId, PlayerRegistry};
pub use partnership::{Partnership, PartnershipDirection};
