use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Normalize a player name for consistent matching
pub fn normalize_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Unique identifier for a player across sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerId {
    /// Canonical name (normalized, lowercase)
    pub canonical_name: String,
    /// ACBL number if known
    pub acbl_number: Option<String>,
}

impl PlayerId {
    /// Create a new PlayerId from a raw name
    pub fn from_name(name: &str) -> Self {
        Self {
            canonical_name: normalize_name(name),
            acbl_number: None,
        }
    }

    /// Create a new PlayerId with an ACBL number
    pub fn with_acbl_number(name: &str, acbl_number: Option<String>) -> Self {
        Self {
            canonical_name: normalize_name(name),
            acbl_number,
        }
    }

    /// Get a display name (title case)
    pub fn display_name(&self) -> String {
        self.canonical_name
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl PartialEq for PlayerId {
    fn eq(&self, other: &Self) -> bool {
        // If both have ACBL numbers, compare by that
        if let (Some(ref a), Some(ref b)) = (&self.acbl_number, &other.acbl_number) {
            return a == b;
        }
        // Otherwise compare by canonical name
        self.canonical_name == other.canonical_name
    }
}

impl Eq for PlayerId {}

impl Hash for PlayerId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash by ACBL number if available, otherwise by name
        if let Some(ref acbl) = self.acbl_number {
            acbl.hash(state);
        } else {
            self.canonical_name.hash(state);
        }
    }
}

/// ACBL masterpoint information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterpointInfo {
    pub total_points: f64,
    pub rank: String,
    pub location: Option<String>,
}

/// Complete player information aggregated from all sources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    /// All name variations seen for this player
    pub name_variants: Vec<String>,
    /// ACBL masterpoint info if available
    pub masterpoints: Option<MasterpointInfo>,
}

impl Player {
    /// Create a new player from an ID
    pub fn new(id: PlayerId) -> Self {
        Self {
            id,
            name_variants: Vec::new(),
            masterpoints: None,
        }
    }

    /// Add a name variant
    pub fn add_name_variant(&mut self, name: &str) {
        let name = name.trim().to_string();
        if !name.is_empty() && !self.name_variants.contains(&name) {
            self.name_variants.push(name);
        }
    }

    /// Get the best display name (first variant or canonical)
    pub fn display_name(&self) -> String {
        self.name_variants
            .first()
            .cloned()
            .unwrap_or_else(|| self.id.display_name())
    }
}

/// Registry of all players in a game
#[derive(Debug, Default)]
pub struct PlayerRegistry {
    players: HashMap<PlayerId, Player>,
}

impl PlayerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a player by name
    pub fn get_or_create(&mut self, name: &str, acbl_number: Option<String>) -> PlayerId {
        let id = PlayerId::with_acbl_number(name, acbl_number);

        if !self.players.contains_key(&id) {
            let mut player = Player::new(id.clone());
            player.add_name_variant(name);
            self.players.insert(id.clone(), player);
        } else if let Some(player) = self.players.get_mut(&id) {
            player.add_name_variant(name);
        }

        id
    }

    /// Get a player by ID
    pub fn get(&self, id: &PlayerId) -> Option<&Player> {
        self.players.get(id)
    }

    /// Get all players
    pub fn all_players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    /// Number of players
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}
