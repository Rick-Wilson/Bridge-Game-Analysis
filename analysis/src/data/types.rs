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

/// One par-contract entry. Multiple are possible when more than one
/// declarer/strain ties for the optimal score (e.g. N 4H= and S 4S=
/// both score 420 NS).
#[derive(Debug, Clone)]
pub struct ParContract {
    /// Signed integer; positive = NS gain.
    pub score: i32,
    pub contract: ParsedContract,
    pub declarer: Direction,
    /// Tricks relative to contract (+1, -2, 0 for "="). May be None
    /// for sources that didn't carry trick info; in that case the
    /// builder fills it in by inverting the score.
    pub tricks_relative: Option<i32>,
}

impl ParContract {
    /// Side that declares this par contract ("NS" or "EW").
    pub fn side(&self) -> &'static str {
        match self.declarer {
            Direction::North | Direction::South => "NS",
            Direction::East | Direction::West => "EW",
        }
    }

    /// Render in the historical PBN-style display, e.g. "EW 3NT+2",
    /// "NS 4S=", or "EW 6SX-1". Strain is always two letters for NT.
    pub fn display(&self) -> String {
        let strain_str = match self.contract.strain {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "NT",
        };
        let doubled_str = match self.contract.doubled {
            Doubled::None => "",
            Doubled::Doubled => "X",
            Doubled::Redoubled => "XX",
        };
        let suffix = match self.tricks_relative {
            Some(0) => "=".to_string(),
            Some(n) if n > 0 => format!("+{}", n),
            Some(n) => format!("{}", n),
            None => String::new(),
        };
        format!(
            "{} {}{}{}{}",
            self.side(),
            self.contract.level,
            strain_str,
            doubled_str,
            suffix
        )
    }

    /// Render the score line like "EW 460" or "NS 420".
    pub fn score_display(&self) -> String {
        if self.score >= 0 {
            format!("NS {}", self.score)
        } else {
            format!("EW {}", -self.score)
        }
    }
}

/// Render a slice of par contracts as a single contract string and a single
/// score string (the score is the same across all entries by definition).
/// Used by adapters that need the historical "EW 3N+2; N 4H=" / "EW 460"
/// display pair from typed data.
pub fn render_par_display(par: &[ParContract]) -> (Option<String>, Option<String>) {
    if par.is_empty() {
        return (None, None);
    }
    let contracts = par
        .iter()
        .map(|p| p.display())
        .collect::<Vec<_>>()
        .join("; ");
    let score = par[0].score_display();
    (Some(contracts), Some(score))
}

/// Per-declarer × per-strain double-dummy trick counts.
///
/// Each entry says "if this declarer played in this strain, double-dummy
/// optimal play makes N tricks". `None` for a strain means the source
/// didn't disambiguate that count (e.g., ACBL Live collapses 0–6 tricks
/// into a single bucket).
pub type DoubleDummyTricks = HashMap<Direction, DdStrains>;

/// Trick counts by strain for one declarer.
#[derive(Debug, Clone, Default)]
pub struct DdStrains {
    pub clubs: Option<u8>,
    pub diamonds: Option<u8>,
    pub hearts: Option<u8>,
    pub spades: Option<u8>,
    pub no_trump: Option<u8>,
}

impl DdStrains {
    /// Look up tricks for a strain.
    pub fn get(&self, strain: Strain) -> Option<u8> {
        match strain {
            Strain::Clubs => self.clubs,
            Strain::Diamonds => self.diamonds,
            Strain::Hearts => self.hearts,
            Strain::Spades => self.spades,
            Strain::NoTrump => self.no_trump,
        }
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
    /// Double dummy tricks per declarer/strain (from PBN or schema input)
    pub double_dummy: Option<DoubleDummyTricks>,
    /// Par contracts. Empty when no par data is available. May contain
    /// more than one entry when ties exist (e.g. N 4H= and S 4S=).
    pub par: Vec<ParContract>,
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

        let double_dummy = board
            .double_dummy_tricks
            .as_deref()
            .and_then(parse_pbn_dd_string);

        let par = parse_pbn_par(
            board.par_contract.as_deref(),
            board.optimum_score.as_deref(),
        );

        Self {
            number: board.number.unwrap_or(0),
            dealer: board.dealer.unwrap_or(Direction::North),
            vulnerability: board.vulnerable,
            deal,
            double_dummy,
            par,
        }
    }

    /// Look up double-dummy tricks for a given declarer direction and strain.
    pub fn dd_tricks(&self, declarer: Direction, strain: Strain) -> Option<u8> {
        self.double_dummy.as_ref()?.get(&declarer)?.get(strain)
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

/// Parse PBN par strings into typed `ParContract` entries.
///
/// `par_str` is the raw `[OptimumResultTable]` / `[ParContract]` value, e.g.:
/// - `"NS 3H="` — NS plays 3H making
/// - `"EW 3N+2"` — EW plays 3NT making 5
/// - `"N 4H=; S 4S="` — tie between two declarers (semicolon or comma separated)
/// - `"EW 7S="` — EW plays 7S making
///
/// `score_str` is the raw `[OptimumScore]` value, e.g. `"EW 460"`. The side
/// determines the sign of the resulting NS-perspective score; the magnitude
/// is shared across all par entries.
///
/// Returns an empty Vec if no par entries can be parsed.
fn parse_pbn_par(par_str: Option<&str>, score_str: Option<&str>) -> Vec<ParContract> {
    let par_str = match par_str {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Vec::new(),
    };

    // Score: "EW 460" -> -460 from NS perspective; "NS 460" -> +460.
    let signed_score: i32 = score_str
        .map(|s| {
            let mut tokens = s.split_whitespace();
            let side = tokens.next().unwrap_or("");
            let magnitude: i32 = tokens.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            match side {
                "NS" | "N" | "S" => magnitude,
                "EW" | "E" | "W" => -magnitude,
                _ => 0,
            }
        })
        .unwrap_or(0);

    let mut out = Vec::new();
    for part in par_str.split([';', ',']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut tokens = part.split_whitespace();
        let side = match tokens.next() {
            Some(s) => s,
            None => continue,
        };
        let body = match tokens.next() {
            Some(s) => s,
            None => continue,
        };

        // Split body into "<level><strain>[double]" + "<result>"
        let (contract_part, result_part) = split_contract_and_result(body);

        let contract = match ParsedContract::parse(contract_part) {
            Some(c) => c,
            None => continue,
        };

        // Pick a canonical declarer for the side. The PBN side string can be
        // "NS"/"EW" (no specific seat) or a single seat letter "N"/"E"/"S"/"W".
        let declarer = match side {
            "N" => Direction::North,
            "E" => Direction::East,
            "S" => Direction::South,
            "W" => Direction::West,
            "NS" => Direction::North,
            "EW" => Direction::East,
            _ => continue,
        };

        let tricks_relative = parse_par_result(result_part);

        out.push(ParContract {
            score: signed_score,
            contract,
            declarer,
            tricks_relative,
        });
    }
    out
}

/// Split a par body like "3NT+2", "4SX-1", or "4H=" into (contract, result).
/// The result is a string starting with `=`, `+N`, or `-N`; empty if absent.
fn split_contract_and_result(body: &str) -> (&str, &str) {
    if let Some(idx) = body.find(['=', '+', '-']) {
        body.split_at(idx)
    } else {
        (body, "")
    }
}

/// Parse the trick-suffix part of a par entry: "=" → 0, "+2" → 2, "-1" → -1.
fn parse_par_result(s: &str) -> Option<i32> {
    let s = s.trim();
    if s == "=" {
        return Some(0);
    }
    s.parse::<i32>().ok()
}

/// Parse a PBN `[DoubleDummyTricks "..."]` value into the typed map.
///
/// PBN format: 20 hex chars, layout NT,S,H,D,C × N,S,E,W. Each hex char
/// (`0`–`9`, `a`–`d`) represents tricks 0–13. Returns `None` if the string
/// is shorter than expected; otherwise produces a fully-populated map with
/// every declarer/strain filled in.
fn parse_pbn_dd_string(s: &str) -> Option<DoubleDummyTricks> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let parse_nibble = |idx: usize| -> Option<u8> {
        match bytes.get(idx)? {
            ch @ b'0'..=b'9' => Some(ch - b'0'),
            ch @ b'a'..=b'd' => Some(ch - b'a' + 10),
            ch @ b'A'..=b'D' => Some(ch - b'A' + 10),
            _ => None,
        }
    };
    let strains_for = |dir_offset: usize| DdStrains {
        no_trump: parse_nibble(dir_offset),
        spades: parse_nibble(dir_offset + 1),
        hearts: parse_nibble(dir_offset + 2),
        diamonds: parse_nibble(dir_offset + 3),
        clubs: parse_nibble(dir_offset + 4),
    };
    let mut dd = DoubleDummyTricks::new();
    dd.insert(Direction::North, strains_for(0));
    dd.insert(Direction::South, strains_for(5));
    dd.insert(Direction::East, strains_for(10));
    dd.insert(Direction::West, strains_for(15));
    Some(dd)
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
#[derive(Debug, Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// PBN's 20-char DD layout is N,S,E,W × NT,S,H,D,C. The string
    /// "abcd9 abcd9 12345 67890" (spaces only here for clarity) means:
    ///   N: NT=10 S=11 H=12 D=13 C=9
    ///   S: NT=10 S=11 H=12 D=13 C=9
    ///   E: NT=1  S=2  H=3  D=4  C=5
    ///   W: NT=6  S=7  H=8  D=9  C=0
    #[test]
    fn parses_pbn_dd_string() {
        let dd = parse_pbn_dd_string("abcd9abcd9123456789a").expect("should parse");
        // Last char is 'a' = 10 to keep all values legal (0..=13)
        assert_eq!(dd.get(&Direction::North).unwrap().no_trump, Some(10));
        assert_eq!(dd.get(&Direction::North).unwrap().clubs, Some(9));
        assert_eq!(dd.get(&Direction::South).unwrap().diamonds, Some(13));
        assert_eq!(dd.get(&Direction::East).unwrap().no_trump, Some(1));
        assert_eq!(dd.get(&Direction::West).unwrap().clubs, Some(10));
    }

    #[test]
    fn rejects_short_pbn_dd_string() {
        assert!(parse_pbn_dd_string("abc").is_none());
    }

    #[test]
    fn parses_pbn_par_simple() {
        let par = parse_pbn_par(Some("EW 3N+2"), Some("EW 460"));
        assert_eq!(par.len(), 1);
        assert_eq!(par[0].score, -460);
        assert_eq!(par[0].contract.level, 3);
        assert_eq!(par[0].contract.strain, Strain::NoTrump);
        assert_eq!(par[0].declarer, Direction::East);
        assert_eq!(par[0].tricks_relative, Some(2));
        assert_eq!(par[0].display(), "EW 3NT+2");
        assert_eq!(par[0].score_display(), "EW 460");
    }

    #[test]
    fn parses_pbn_par_tied() {
        let par = parse_pbn_par(Some("N 4H=; S 4S="), Some("NS 420"));
        assert_eq!(par.len(), 2);
        assert_eq!(par[0].contract.strain, Strain::Hearts);
        assert_eq!(par[0].declarer, Direction::North);
        assert_eq!(par[1].contract.strain, Strain::Spades);
        assert_eq!(par[1].declarer, Direction::South);
        assert!(par.iter().all(|p| p.score == 420));
        let (display, score) = render_par_display(&par);
        assert_eq!(display.unwrap(), "NS 4H=; NS 4S=");
        assert_eq!(score.unwrap(), "NS 420");
    }

    #[test]
    fn parses_pbn_par_empty_inputs() {
        assert!(parse_pbn_par(None, None).is_empty());
        assert!(parse_pbn_par(Some(""), None).is_empty());
    }

    /// dd_tricks() preserves the same lookup semantics as before the typed conversion.
    #[test]
    fn dd_tricks_lookup() {
        let dd = parse_pbn_dd_string("abcd9abcd9123456789a").expect("should parse");
        let board = BoardData {
            number: 1,
            dealer: Direction::North,
            vulnerability: Vulnerability::None,
            deal: None,
            double_dummy: Some(dd),
            par: Vec::new(),
        };
        assert_eq!(board.dd_tricks(Direction::North, Strain::NoTrump), Some(10));
        assert_eq!(board.dd_tricks(Direction::East, Strain::Spades), Some(2));
        assert_eq!(board.dd_tricks(Direction::West, Strain::Clubs), Some(10));
    }
}
