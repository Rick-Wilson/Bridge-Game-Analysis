use crate::identity::{Partnership, PartnershipDirection, PlayerId, PlayerRegistry};
use bridge_parsers::{
    Board, Contract, Deal, Direction, Doubled, Hand, Strain, Suit, Vulnerability,
};
use std::collections::HashMap;

/// A parsed contract with all relevant information
#[derive(Debug, Clone)]
pub struct ParsedContract {
    pub level: u8,
    pub strain: Strain,
    pub doubled: Doubled,
}

impl ParsedContract {
    /// Create from bridge_parsers Contract
    pub fn from_contract(c: &Contract) -> Self {
        Self {
            level: c.level,
            strain: c.strain,
            doubled: c.doubled,
        }
    }

    /// Parse from a contract string like "3NT", "4S", "2HX"
    pub fn parse(s: &str) -> Option<Self> {
        Contract::parse(s).map(|c| Self::from_contract(&c))
    }

    /// Calculate the number of tricks required to make the contract
    pub fn tricks_required(&self) -> u8 {
        self.level + 6
    }

    /// Get display string
    pub fn display(&self) -> String {
        let strain_str = match self.strain {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "NT",
        };
        let doubled_str = match self.doubled {
            Doubled::None => "",
            Doubled::Doubled => "X",
            Doubled::Redoubled => "XX",
        };
        format!("{}{}{}", self.level, strain_str, doubled_str)
    }

    /// Get LIN format bid string (e.g., "4S", "3N" for notrump)
    pub fn lin_bid(&self) -> String {
        let strain_str = match self.strain {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "N", // LIN uses "N" not "NT"
        };
        format!("{}{}", self.level, strain_str)
    }
}

/// Complete data for a single board
#[derive(Debug, Clone)]
pub struct BoardData {
    pub number: u32,
    pub dealer: Direction,
    pub vulnerability: Vulnerability,
    /// The deal (all four hands)
    pub deal: Option<Deal>,
    /// Double dummy tricks string (if available from PBN)
    pub double_dummy_tricks: Option<String>,
    /// Par contract string (if available from PBN)
    pub par_contract: Option<String>,
    /// Optimum score (if available from PBN)
    pub optimum_score: Option<String>,
}

impl BoardData {
    /// Create from a bridge_parsers Board
    pub fn from_board(board: &Board) -> Self {
        // Only store deal if it has cards
        let deal = if board.deal.has_cards() {
            Some(board.deal.clone())
        } else {
            None
        };

        Self {
            number: board.number.unwrap_or(0),
            dealer: board.dealer.unwrap_or(Direction::North),
            vulnerability: board.vulnerable,
            deal,
            double_dummy_tricks: board.double_dummy_tricks.clone(),
            par_contract: board.par_contract.clone(),
            optimum_score: board.optimum_score.clone(),
        }
    }

    /// Look up double-dummy tricks for a given declarer direction and strain.
    ///
    /// The DD tricks string is 20 hex chars: 4 directions (N,S,E,W) × 5 strains (NT,S,H,D,C).
    /// Each hex char (0-9, a-d) represents tricks 0-13.
    pub fn dd_tricks(&self, declarer: Direction, strain: Strain) -> Option<u8> {
        let dd = self.double_dummy_tricks.as_ref()?;
        if dd.len() < 20 {
            return None;
        }

        let dir_offset = match declarer {
            Direction::North => 0,
            Direction::South => 5,
            Direction::East => 10,
            Direction::West => 15,
        };
        let strain_offset = match strain {
            Strain::NoTrump => 0,
            Strain::Spades => 1,
            Strain::Hearts => 2,
            Strain::Diamonds => 3,
            Strain::Clubs => 4,
        };

        let ch = dd.as_bytes().get(dir_offset + strain_offset)?;
        match ch {
            b'0'..=b'9' => Some(ch - b'0'),
            b'a'..=b'd' => Some(ch - b'a' + 10),
            b'A'..=b'D' => Some(ch - b'A' + 10),
            _ => None,
        }
    }

    /// Check if declarer is vulnerable
    pub fn is_declarer_vulnerable(&self, declarer: Direction) -> bool {
        self.vulnerability.is_vulnerable(declarer)
    }

    /// Generate a BBO hand viewer URL for this board
    /// Optionally include player names (S, W, N, E order) and contract result
    pub fn bbo_handviewer_url(
        &self,
        players: Option<&SeatPlayers>,
        contract_result: Option<&ContractResult>,
    ) -> Option<String> {
        let deal = self.deal.as_ref()?;

        // Build LIN format: pn|S,W,N,E|md|dealer + hands in S,W,N,E order|sv|vul|ah|Board X|
        let dealer_digit = match self.dealer {
            Direction::South => '1',
            Direction::West => '2',
            Direction::North => '3',
            Direction::East => '4',
        };

        // Format hands in S,W,N,E order (BBO convention)
        let south_hand = format_hand_lin(deal.hand(Direction::South));
        let west_hand = format_hand_lin(deal.hand(Direction::West));
        let north_hand = format_hand_lin(deal.hand(Direction::North));
        // East hand is calculated by BBO, so we can leave it empty

        let vul_str = match self.vulnerability {
            Vulnerability::None => "o",
            Vulnerability::NorthSouth => "n",
            Vulnerability::EastWest => "e",
            Vulnerability::Both => "b",
        };

        // Build player names section if provided
        let pn_section = if let Some(p) = players {
            // LIN uses + for spaces in names
            let s = p.south.replace(' ', "+");
            let w = p.west.replace(' ', "+");
            let n = p.north.replace(' ', "+");
            let e = p.east.replace(' ', "+");
            format!("pn|{},{},{},{}|", s, w, n, e)
        } else {
            String::new()
        };

        // Build bidding section if contract provided
        let bidding_section = if let Some(cr) = contract_result {
            build_bidding_lin(self.dealer, cr)
        } else {
            String::new()
        };

        let lin = format!(
            "{}md|{}{},{},{},|sv|{}|ah|Board+{}|{}",
            pn_section,
            dealer_digit,
            south_hand,
            west_hand,
            north_hand,
            vul_str,
            self.number,
            bidding_section
        );

        // URL encode the LIN string
        let encoded = urlencoding::encode(&lin);
        Some(format!(
            "https://www.bridgebase.com/tools/handviewer.html?lin={}",
            encoded
        ))
    }
}

/// Player names for each seat (used for BBO hand viewer URLs)
#[derive(Debug, Clone)]
pub struct SeatPlayers {
    pub north: String,
    pub east: String,
    pub south: String,
    pub west: String,
}

/// Contract result info for BBO hand viewer URLs
#[derive(Debug, Clone)]
pub struct ContractResult {
    pub contract: ParsedContract,
    pub declarer: Direction,
}

impl SeatPlayers {
    /// Create from NS and EW partnerships using seat-based ordering.
    /// NS pair: first_player() = North, second_player() = South.
    /// EW pair: first_player() = West, second_player() = East.
    pub fn from_partnerships(ns_pair: &Partnership, ew_pair: &Partnership) -> Self {
        Self {
            north: ns_pair.first_player().display_name(),
            south: ns_pair.second_player().display_name(),
            west: ew_pair.first_player().display_name(),
            east: ew_pair.second_player().display_name(),
        }
    }
}

/// Format a hand in LIN format (SHDC order, suit letter then cards)
fn format_hand_lin(hand: &Hand) -> String {
    let mut result = String::new();

    for suit in [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs] {
        result.push(match suit {
            Suit::Spades => 'S',
            Suit::Hearts => 'H',
            Suit::Diamonds => 'D',
            Suit::Clubs => 'C',
        });
        for card in hand.cards_in_suit(suit) {
            result.push(card.rank.to_char());
        }
    }

    result
}

/// Build a LIN bidding sequence from dealer to final contract
/// Generates a minimal auction: passes until declarer, then contract bid, then passes
fn build_bidding_lin(dealer: Direction, cr: &ContractResult) -> String {
    let mut bids = Vec::new();
    let directions = [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ];

    // Find dealer position in the rotation
    let dealer_idx = directions.iter().position(|&d| d == dealer).unwrap_or(0);
    let declarer_idx = directions
        .iter()
        .position(|&d| d == cr.declarer)
        .unwrap_or(0);

    // Add passes from dealer until declarer
    let mut current_idx = dealer_idx;
    while current_idx != declarer_idx {
        bids.push("mb|p|".to_string());
        current_idx = (current_idx + 1) % 4;
    }

    // Declarer bids the contract
    bids.push(format!("mb|{}|", cr.contract.lin_bid()));

    // Handle doubled/redoubled
    match cr.contract.doubled {
        Doubled::None => {
            // Three passes to end auction
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
        Doubled::Doubled => {
            // Next opponent doubles, then two passes
            bids.push("mb|d|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
        Doubled::Redoubled => {
            // Opponent doubles, declarer's partner redoubles, two passes
            bids.push("mb|d|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|r|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
    }

    bids.join("")
}

/// A single board result (one table's play of one board)
#[derive(Debug, Clone)]
pub struct BoardResult {
    pub board_number: u32,
    pub section: i32,
    pub table: i32,
    pub round: i32,
    /// NS partnership
    pub ns_pair: Partnership,
    /// EW partnership
    pub ew_pair: Partnership,
    /// Who declared
    pub declarer_direction: Direction,
    /// Declarer's player ID
    pub declarer: PlayerId,
    /// The contract played (None if passed out)
    pub contract: Option<ParsedContract>,
    /// Tricks relative to contract (+1, -2, etc.)
    pub tricks_relative: Option<i32>,
    /// NS score (negative means EW scored)
    pub ns_score: i32,
    /// Lead card if recorded
    pub lead_card: Option<String>,
}

impl BoardResult {
    /// Which partnership declared this board
    pub fn declaring_partnership(&self) -> &Partnership {
        match self.declarer_direction {
            Direction::North | Direction::South => &self.ns_pair,
            Direction::East | Direction::West => &self.ew_pair,
        }
    }

    /// Which direction the declaring partnership was sitting
    pub fn declaring_direction(&self) -> PartnershipDirection {
        match self.declarer_direction {
            Direction::North | Direction::South => PartnershipDirection::NorthSouth,
            Direction::East | Direction::West => PartnershipDirection::EastWest,
        }
    }

    /// Calculate absolute tricks made (0-13)
    pub fn tricks_made(&self) -> Option<u8> {
        let contract = self.contract.as_ref()?;
        let relative = self.tricks_relative?;
        let made = contract.level as i32 + 6 + relative;
        Some(made.clamp(0, 13) as u8)
    }
}

/// Complete merged data for a game session
#[derive(Debug)]
pub struct GameData {
    /// Event/session name (e.g., "Monday Morning Pairs")
    pub event_name: Option<String>,
    /// Event date string
    pub event_date: Option<String>,
    /// Board information keyed by board number
    pub boards: HashMap<u32, BoardData>,
    /// Player registry
    pub players: PlayerRegistry,
    /// All board results
    pub results: Vec<BoardResult>,
    /// Pair-number → (first_player, second_player) in display order.
    /// Key is (section, pair_number). Populated from RoundData round 1.
    /// Enables name-override lookup by pair number from pasted ACBL Live data.
    pub pairs_by_number:
        HashMap<(i32, i32), (crate::identity::PlayerId, crate::identity::PlayerId)>,
}

impl GameData {
    /// Create empty game data
    pub fn new() -> Self {
        Self {
            event_name: None,
            event_date: None,
            boards: HashMap::new(),
            players: PlayerRegistry::new(),
            results: Vec::new(),
            pairs_by_number: HashMap::new(),
        }
    }

    /// Get all unique partnerships that played
    pub fn partnerships(&self) -> Vec<Partnership> {
        let mut seen = std::collections::HashSet::new();
        let mut partnerships = Vec::new();

        for result in &self.results {
            if seen.insert(result.ns_pair.clone()) {
                partnerships.push(result.ns_pair.clone());
            }
            if seen.insert(result.ew_pair.clone()) {
                partnerships.push(result.ew_pair.clone());
            }
        }

        partnerships
    }

    /// Get results for a specific board
    pub fn results_for_board(&self, board_number: u32) -> Vec<&BoardResult> {
        self.results
            .iter()
            .filter(|r| r.board_number == board_number)
            .collect()
    }

    /// Get all board numbers in order
    pub fn board_numbers(&self) -> Vec<u32> {
        let mut numbers: Vec<u32> = self.boards.keys().copied().collect();
        numbers.sort();
        numbers
    }
}

impl Default for GameData {
    fn default() -> Self {
        Self::new()
    }
}
