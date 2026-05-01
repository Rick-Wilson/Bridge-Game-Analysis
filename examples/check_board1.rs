// Quick check of board 1 results
use bridge_club_analysis::{load_game_data, PartnershipDirection};
use std::path::Path;

fn main() {
    let bws_path = Path::new("tests/integration/source/LBC-2026-01-26.bws");
    let pbn_path = Path::new("tests/integration/source/LBC-2026-01-26.pbn");
    
    let data = load_game_data(bws_path, Some(pbn_path), None).unwrap();
    
    println!("Board 1 Results:");
    println!("{:>25} {:>25} {:>10} {:>8}", "NS Pair", "EW Pair", "NS Score", "Contract");
    println!("{}", "-".repeat(75));
    
    for r in data.results.iter().filter(|r| r.board_number == 1) {
        let ns_names = format!("{} - {}", 
            r.ns_pair.player1.display_name(), 
            r.ns_pair.player2.display_name());
        let ew_names = format!("{} - {}", 
            r.ew_pair.player1.display_name(), 
            r.ew_pair.player2.display_name());
        let contract = r.contract.as_ref().map(|c| c.display()).unwrap_or_default();
        let result = r.tricks_relative.map(|t| {
            if t > 0 { format!("+{}", t) }
            else if t < 0 { format!("{}", t) }
            else { "=".to_string() }
        }).unwrap_or_default();
        println!("{:>25} {:>25} {:>10} {:>8}", ns_names, ew_names, r.ns_score, format!("{}{}", contract, result));
    }
}
