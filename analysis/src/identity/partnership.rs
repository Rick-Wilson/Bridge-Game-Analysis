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
    /// First player (alphabetically by canonical name, for consistent Eq/Hash)
    pub player1: PlayerId,
    /// Second player (alphabetically by canonical name)
    pub player2: PlayerId,
    /// Whether player1 should be displayed first.
    /// When false, display order is reversed from alphabetical (player2 first).
    /// Used for ACBL seat ordering: N-S pairs show North first, E-W pairs show West first.
    #[serde(skip)]
    player1_displays_first: bool,
}

impl Partnership {
    /// Create a new partnership with consistent ordering (alphabetical display)
    pub fn new(p1: PlayerId, p2: PlayerId) -> Self {
        // Order by canonical name for consistent hashing/comparison
        if p1.canonical_name <= p2.canonical_name {
            Self {
                player1: p1,
                player2: p2,
                player1_displays_first: true,
            }
        } else {
            Self {
                player1: p2,
                player2: p1,
                player1_displays_first: true,
            }
        }
    }

    /// Create a new partnership with seat-based display ordering.
    ///
    /// `display_first` is the player that should appear first in display output
    /// (North for N-S pairs, West for E-W pairs).
    /// Internal storage remains alphabetically sorted for consistent Eq/Hash.
    pub fn new_seated(p1: PlayerId, p2: PlayerId, display_first: &PlayerId) -> Self {
        if p1.canonical_name <= p2.canonical_name {
            Self {
                player1_displays_first: p1 == *display_first,
                player1: p1,
                player2: p2,
            }
        } else {
            Self {
                player1_displays_first: p2 == *display_first,
                player1: p2,
                player2: p1,
            }
        }
    }

    /// Check if a player is in this partnership
    pub fn contains(&self, player: &PlayerId) -> bool {
        &self.player1 == player || &self.player2 == player
    }

    /// Get the player displayed first (seat-based: N for N-S, W for E-W)
    pub fn first_player(&self) -> &PlayerId {
        if self.player1_displays_first {
            &self.player1
        } else {
            &self.player2
        }
    }

    /// Get the player displayed second (seat-based: S for N-S, E for E-W)
    pub fn second_player(&self) -> &PlayerId {
        if self.player1_displays_first {
            &self.player2
        } else {
            &self.player1
        }
    }

    /// Get display name for the partnership in seat order (short format: "First L. - First L.")
    pub fn display_name(&self) -> String {
        format!(
            "{} - {}",
            self.first_player().short_name(),
            self.second_player().short_name()
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
