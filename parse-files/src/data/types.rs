//! Bridge-domain types used by the BWS/PBN adapter and the schema-walk
//! enrich passes.
//!
//! Pre-2026-04: this module also held the analysis-shaped types
//! (GameData, BoardResult, PlayerRegistry, Par/DoubleDummy maps, etc.).
//! Those have been retired with the analyzer; what remains is just the
//! intermediate types the adapter and the handviewer-URL builder use:
//!
//!   - [`ParsedContract`] — strict canonical-form contract parser.
//!   - [`BoardData`] — minimal "deal + dealer + vulnerability" struct
//!     used to drive [`BoardData::bbo_handviewer_url`].
//!   - [`SeatPlayers`], [`ContractResult`] — args to that method.

use bridge_parsers::{Contract, Deal, Direction, Doubled, Hand, Strain, Suit, Vulnerability};

/// A parsed contract with all relevant information.
#[derive(Debug, Clone)]
pub struct ParsedContract {
    pub level: u8,
    pub strain: Strain,
    pub doubled: Doubled,
}

impl ParsedContract {
    /// Create from bridge_parsers Contract.
    pub fn from_contract(c: &Contract) -> Self {
        Self {
            level: c.level,
            strain: c.strain,
            doubled: c.doubled,
        }
    }

    /// Parse from a contract string. Accepts both the canonical no-space
    /// form ("3NT", "4S", "2HX", "6NTXX") and the space-separated form
    /// ("3 NT", "4 S", "2 H X") that the upstream bridge_parsers parser
    /// requires.
    ///
    /// The canonical form is what the schema specifies and what
    /// `display()` emits, so it must round-trip through the schema. We
    /// try canonical first; if that fails we fall through to the
    /// upstream parser for legacy / human-typed input.
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(c) = Self::parse_canonical(s) {
            return Some(c);
        }
        Contract::parse(s).map(|c| Self::from_contract(&c))
    }

    /// Strict parser for the schema's canonical contract string:
    /// `{1-7}{C|D|H|S|NT}{X|XX}?`. Returns None for any deviation; the
    /// public `parse` falls back to the looser upstream parser.
    fn parse_canonical(s: &str) -> Option<Self> {
        let s = s.trim();
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return None;
        }
        let level_byte = *bytes.first()?;
        if !level_byte.is_ascii_digit() {
            return None;
        }
        let level = level_byte - b'0';
        if !(1..=7).contains(&level) {
            return None;
        }
        let rest = &s[1..];
        let (strain, after) = if let Some(stripped) = rest.strip_prefix("NT") {
            (Strain::NoTrump, stripped)
        } else {
            let ch = rest.chars().next()?;
            let strain = match ch {
                'C' => Strain::Clubs,
                'D' => Strain::Diamonds,
                'H' => Strain::Hearts,
                'S' => Strain::Spades,
                'N' => Strain::NoTrump, // bare "N" — tolerate; "NT" already handled above
                _ => return None,
            };
            (strain, &rest[ch.len_utf8()..])
        };
        let doubled = match after {
            "" => Doubled::None,
            "X" => Doubled::Doubled,
            "XX" => Doubled::Redoubled,
            _ => return None,
        };
        Some(Self {
            level,
            strain,
            doubled,
        })
    }

    /// Get display string ("3NT", "4SX", "6CXX").
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

    /// LIN-format bid string (e.g. "4S", "3N" — LIN uses bare "N" for NT).
    pub fn lin_bid(&self) -> String {
        let strain_str = match self.strain {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "N",
        };
        format!("{}{}", self.level, strain_str)
    }
}

/// Minimal board context used to construct a BBO handviewer URL.
///
/// The retired analysis layer carried double-dummy and par on `BoardData`;
/// neither is needed for the URL-building so they're gone. The schema's
/// `Board` type carries dd / par natively for any consumer that wants
/// them.
#[derive(Debug, Clone)]
pub struct BoardData {
    pub number: u32,
    pub dealer: Direction,
    pub vulnerability: Vulnerability,
    /// All four hands, when present.
    pub deal: Option<Deal>,
}

impl BoardData {
    /// Generate a BBO hand-viewer URL for this board. `players` provides
    /// names for the LIN `pn|` section (omitted when None). `contract_result`
    /// generates a constructed auction (passes from dealer to declarer,
    /// the contract bid, then closing passes/X/XX); when None, BBO renders
    /// the deal without an auction.
    pub fn bbo_handviewer_url(
        &self,
        players: Option<&SeatPlayers>,
        contract_result: Option<&ContractResult>,
    ) -> Option<String> {
        let deal = self.deal.as_ref()?;

        let dealer_digit = match self.dealer {
            Direction::South => '1',
            Direction::West => '2',
            Direction::North => '3',
            Direction::East => '4',
        };

        // Format hands in S, W, N, E order (BBO convention). East is
        // computed by BBO from the other three so it's omitted.
        let south_hand = format_hand_lin(deal.hand(Direction::South));
        let west_hand = format_hand_lin(deal.hand(Direction::West));
        let north_hand = format_hand_lin(deal.hand(Direction::North));

        let vul_str = match self.vulnerability {
            Vulnerability::None => "o",
            Vulnerability::NorthSouth => "n",
            Vulnerability::EastWest => "e",
            Vulnerability::Both => "b",
        };

        let pn_section = if let Some(p) = players {
            // LIN uses + for spaces in names.
            let s = p.south.replace(' ', "+");
            let w = p.west.replace(' ', "+");
            let n = p.north.replace(' ', "+");
            let e = p.east.replace(' ', "+");
            format!("pn|{},{},{},{}|", s, w, n, e)
        } else {
            String::new()
        };

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

        let encoded = urlencoding::encode(&lin);
        Some(format!(
            "https://www.bridgebase.com/tools/handviewer.html?lin={}",
            encoded
        ))
    }
}

/// Player names per seat, used by [`BoardData::bbo_handviewer_url`].
#[derive(Debug, Clone)]
pub struct SeatPlayers {
    pub north: String,
    pub east: String,
    pub south: String,
    pub west: String,
}

/// Contract + declarer pair, used by [`BoardData::bbo_handviewer_url`].
#[derive(Debug, Clone)]
pub struct ContractResult {
    pub contract: ParsedContract,
    pub declarer: Direction,
}

// ---- private helpers ----

/// Format a hand in LIN format (SHDC order, suit letter then cards).
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

/// Build a LIN bidding sequence: passes from dealer to declarer, the
/// contract bid, then closing passes (or X/XX for doubled/redoubled).
fn build_bidding_lin(dealer: Direction, cr: &ContractResult) -> String {
    let mut bids = Vec::new();
    let directions = [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ];

    let dealer_idx = directions.iter().position(|&d| d == dealer).unwrap_or(0);
    let declarer_idx = directions
        .iter()
        .position(|&d| d == cr.declarer)
        .unwrap_or(0);

    let mut current_idx = dealer_idx;
    while current_idx != declarer_idx {
        bids.push("mb|p|".to_string());
        current_idx = (current_idx + 1) % 4;
    }

    bids.push(format!("mb|{}|", cr.contract.lin_bid()));

    match cr.contract.doubled {
        Doubled::None => {
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
        Doubled::Doubled => {
            bids.push("mb|d|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
        Doubled::Redoubled => {
            bids.push("mb|d|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|r|".to_string());
            bids.push("mb|p|".to_string());
            bids.push("mb|p|".to_string());
        }
    }

    bids.join("")
}
