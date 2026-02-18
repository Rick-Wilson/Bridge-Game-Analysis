use super::PlayerId;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Direction of a partnership (NS or EW)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PartnershipDirection {
    NorthSouth,
    EastWest,
}

impl PartnershipDirection {
    /// Get the opposite direction
    pub fn opposite(&self) -> Self {
        match self {
            PartnershipDirection::NorthSouth => PartnershipDirection::EastWest,
            PartnershipDirection::EastWest => PartnershipDirection::NorthSouth,
        }
    }
}

impl std::fmt::Display for PartnershipDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartnershipDirection::NorthSouth => write!(f, "N-S"),
            PartnershipDirection::EastWest => write!(f, "E-W"),
        }
    }
}

/// A partnership (two players playing together)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partnership {
    /// First player (alphabetically by canonical name)
    pub player1: PlayerId,
    /// Second player (partner)
    pub player2: PlayerId,
}

impl Partnership {
    /// Create a new partnership with consistent ordering
    pub fn new(p1: PlayerId, p2: PlayerId) -> Self {
        // Order by canonical name for consistent hashing/comparison
        if p1.canonical_name <= p2.canonical_name {
            Self {
                player1: p1,
                player2: p2,
            }
        } else {
            Self {
                player1: p2,
                player2: p1,
            }
        }
    }

    /// Check if a player is in this partnership
    pub fn contains(&self, player: &PlayerId) -> bool {
        &self.player1 == player || &self.player2 == player
    }

    /// Get display name for the partnership
    pub fn display_name(&self) -> String {
        format!(
            "{} - {}",
            self.player1.display_name(),
            self.player2.display_name()
        )
    }
}

impl PartialEq for Partnership {
    fn eq(&self, other: &Self) -> bool {
        // Due to consistent ordering, we can just compare directly
        self.player1 == other.player1 && self.player2 == other.player2
    }
}

impl Eq for Partnership {}

impl Hash for Partnership {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.player1.hash(state);
        self.player2.hash(state);
    }
}
