use anyhow::{Context, Result};
use bridge_club_analysis::{
    analyze_bidding_performance, analyze_board, analyze_declarer_performance, analyze_player,
    load_game_data, normalize_name, Config, ContractResult, Direction, GameData,
    PartnershipDirection, ResultCause, SeatPlayers,
};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::{Path, PathBuf};

/// Create a terminal hyperlink using OSC 8 escape sequence
fn hyperlink(url: &str, text: &str) -> String {
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
}

/// Get the ACBL club results URL for an event
fn event_url(event_id: &str) -> String {
    format!("https://my.acbl.org/club-results/details/{}", event_id)
}

/// Get the ACBL club results board URL
fn board_url(event_id: &str, board_number: u32) -> String {
    format!(
        "https://my.acbl.org/club-results/details/{}/boards/{}",
        event_id, board_number
    )
}

#[derive(Parser)]
#[command(name = "bridge-analysis")]
#[command(about = "Analyze bridge club game performance", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a specific player's board-by-board performance
    Player {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,

        /// PBN file with hand records (defaults to BWS path with .pbn extension)
        #[arg(long)]
        pbn: Option<PathBuf>,

        /// Player name to analyze (required)
        #[arg(long)]
        name: String,

        /// ACBL event ID for hyperlinks (e.g., 1380198)
        #[arg(long)]
        event_id: Option<String>,
    },

    /// Analyze declarer performance
    Declarer {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,

        /// PBN file with hand records (defaults to BWS path with .pbn extension)
        #[arg(long)]
        pbn: Option<PathBuf>,

        /// URL for ACBL masterpoint data (overrides config)
        #[arg(long)]
        masterpoints_url: Option<String>,

        /// Filter by specific player name
        #[arg(long)]
        player: Option<String>,
    },

    /// Analyze partnership bidding accuracy
    Bidding {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,

        /// PBN file with hand records (defaults to BWS path with .pbn extension)
        #[arg(long)]
        pbn: Option<PathBuf>,

        /// URL for ACBL masterpoint data (overrides config)
        #[arg(long)]
        masterpoints_url: Option<String>,

        /// Filter by partnership (comma-separated names, e.g. "John Smith,Jane Doe")
        #[arg(long)]
        partnership: Option<String>,
    },

    /// Generate comprehensive game report
    Report {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,

        /// PBN file with hand records (defaults to BWS path with .pbn extension)
        #[arg(long)]
        pbn: Option<PathBuf>,

        /// URL for ACBL masterpoint data (overrides config)
        #[arg(long)]
        masterpoints_url: Option<String>,

        /// ACBL event ID for hyperlinks (e.g., 1380198)
        #[arg(long)]
        event_id: Option<String>,
    },

    /// Analyze a specific board across all tables
    Board {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,

        /// PBN file with hand records (defaults to BWS path with .pbn extension)
        #[arg(long)]
        pbn: Option<PathBuf>,

        /// Board number to analyze
        #[arg(long)]
        board: u32,

        /// ACBL event ID for hyperlinks (e.g., 1380198)
        #[arg(long)]
        event_id: Option<String>,
    },

    /// List all players in the game
    Players {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,
    },

    /// List all partnerships in the game
    Partnerships {
        /// BWS file with game results (defaults to .bws extension)
        #[arg(long)]
        bws: PathBuf,
    },
}

/// Ensure path has .bws extension
fn resolve_bws_path(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension("bws")
    }
}

/// Derive PBN path from BWS path (same base name, .pbn extension)
fn derive_pbn_path(bws_path: &Path) -> PathBuf {
    bws_path.with_extension("pbn")
}

/// Resolve PBN path: use explicit path if provided, otherwise derive from BWS path
fn resolve_pbn_path(pbn: Option<&Path>, bws_path: &Path) -> PathBuf {
    match pbn {
        Some(p) => p.to_path_buf(),
        None => derive_pbn_path(bws_path),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load().unwrap_or_default();

    match cli.command {
        Commands::Player {
            bws,
            pbn,
            name,
            event_id,
        } => {
            let bws_path = resolve_bws_path(&bws);
            let pbn_path = resolve_pbn_path(pbn.as_deref(), &bws_path);

            let pbn_opt = if pbn_path.exists() {
                Some(pbn_path.as_path())
            } else {
                None
            };
            let data =
                load_game_data(&bws_path, pbn_opt, None).context("Failed to load game data")?;

            cmd_player(&data, &name, event_id.as_deref())?;
        }

        Commands::Declarer {
            bws,
            pbn,
            masterpoints_url,
            player,
        } => {
            let bws_path = resolve_bws_path(&bws);
            let pbn_path = resolve_pbn_path(pbn.as_deref(), &bws_path);
            let mp_url = config.masterpoints_url(masterpoints_url.as_deref());

            let pbn_opt = if pbn_path.exists() {
                Some(pbn_path.as_path())
            } else {
                None
            };
            let data =
                load_game_data(&bws_path, pbn_opt, mp_url).context("Failed to load game data")?;

            cmd_declarer(&data, player.as_deref())?;
        }

        Commands::Bidding {
            bws,
            pbn,
            masterpoints_url,
            partnership,
        } => {
            let bws_path = resolve_bws_path(&bws);
            let pbn_path = resolve_pbn_path(pbn.as_deref(), &bws_path);
            let mp_url = config.masterpoints_url(masterpoints_url.as_deref());

            let data = load_game_data(&bws_path, Some(&pbn_path), mp_url)
                .context("Failed to load game data")?;

            cmd_bidding(&data, partnership.as_deref())?;
        }

        Commands::Report {
            bws,
            pbn,
            masterpoints_url,
            event_id,
        } => {
            let bws_path = resolve_bws_path(&bws);
            let pbn_path = resolve_pbn_path(pbn.as_deref(), &bws_path);
            let mp_url = config.masterpoints_url(masterpoints_url.as_deref());

            let pbn_opt = if pbn_path.exists() {
                Some(pbn_path.as_path())
            } else {
                None
            };
            let data =
                load_game_data(&bws_path, pbn_opt, mp_url).context("Failed to load game data")?;

            cmd_report(&data, event_id.as_deref())?;
        }

        Commands::Board {
            bws,
            pbn,
            board,
            event_id,
        } => {
            let bws_path = resolve_bws_path(&bws);
            let pbn_path = resolve_pbn_path(pbn.as_deref(), &bws_path);

            let pbn_opt = if pbn_path.exists() {
                Some(pbn_path.as_path())
            } else {
                None
            };
            let data =
                load_game_data(&bws_path, pbn_opt, None).context("Failed to load game data")?;

            cmd_board(&data, board, event_id.as_deref())?;
        }

        Commands::Players { bws } => {
            let bws_path = resolve_bws_path(&bws);
            let data = load_game_data(&bws_path, None, None).context("Failed to load game data")?;
            cmd_players(&data)?;
        }

        Commands::Partnerships { bws } => {
            let bws_path = resolve_bws_path(&bws);
            let data = load_game_data(&bws_path, None, None).context("Failed to load game data")?;
            cmd_partnerships(&data)?;
        }
    }

    Ok(())
}

/// Color a value based on whether it's positive, negative, or neutral
#[allow(dead_code)]
fn color_value(value: f64, format_str: &str) -> String {
    let formatted = format_str.to_string();
    if value > 0.01 {
        formatted.green().to_string()
    } else if value < -0.01 {
        formatted.red().to_string()
    } else {
        formatted
    }
}

/// Color a matchpoint percentage (>60 green, <40 red)
fn color_mp(pct: f64) -> String {
    let formatted = format!("{:>6.1}%", pct);
    if pct >= 60.0 {
        formatted.green().to_string()
    } else if pct <= 40.0 {
        formatted.red().to_string()
    } else {
        formatted.to_string()
    }
}

/// Color a score based on sign
fn color_score(score: i32) -> String {
    let formatted = format!("{:>7}", score);
    if score > 0 {
        formatted.green().to_string()
    } else if score < 0 {
        formatted.red().to_string()
    } else {
        formatted.to_string()
    }
}

fn cmd_player(data: &GameData, player_name: &str, event_id: Option<&str>) -> Result<()> {
    let analysis = analyze_player(data, player_name);

    match analysis {
        None => {
            println!("{}", format!("Player '{}' not found.", player_name).red());
            return Ok(());
        }
        Some(analysis) => {
            println!(
                "{} {}",
                "Player Analysis:".cyan().bold(),
                analysis.player_name.white().bold()
            );
            println!("{}", "=".repeat(50).cyan());

            // Show event link if event_id provided
            if let Some(eid) = event_id {
                let url = event_url(eid);
                println!("{}", hyperlink(&url, "View event results on ACBL").cyan());
            }
            println!();

            // Summary
            println!("{}", "Summary".cyan().bold());
            println!("{}", "-------".cyan());

            // Collect unique partners and seats
            let mut partners: Vec<String> = analysis
                .board_results
                .iter()
                .map(|br| br.partner.display_name())
                .collect();
            partners.sort();
            partners.dedup();
            let partner_str = partners.join(", ");

            // Collect known seats (from boards where player declared or was dummy)
            let mut seats: Vec<Direction> = analysis
                .board_results
                .iter()
                .filter_map(|br| br.seat)
                .collect();
            seats.sort_by_key(|d| match d {
                Direction::North => 0,
                Direction::East => 1,
                Direction::South => 2,
                Direction::West => 3,
            });
            seats.dedup();
            let seat_str = if seats.is_empty() {
                // Fall back to partnership direction if no specific seat known
                let ns_count = analysis
                    .board_results
                    .iter()
                    .filter(|br| br.direction == PartnershipDirection::NorthSouth)
                    .count();
                if ns_count > 0 {
                    "North-South".to_string()
                } else {
                    "East-West".to_string()
                }
            } else {
                seats
                    .iter()
                    .map(|d| match d {
                        Direction::North => "North",
                        Direction::East => "East",
                        Direction::South => "South",
                        Direction::West => "West",
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            println!("Partner:          {}", partner_str.white());
            println!("Seat:             {}", seat_str.white());
            println!(
                "Boards played:    {}",
                analysis.boards_played.to_string().white()
            );
            println!(
                "Boards declared:  {}",
                analysis.boards_declared.to_string().white()
            );

            let mp_color = if analysis.avg_matchpoint_pct >= 55.0 {
                format!("{:.1}%", analysis.avg_matchpoint_pct).green()
            } else if analysis.avg_matchpoint_pct <= 45.0 {
                format!("{:.1}%", analysis.avg_matchpoint_pct).red()
            } else {
                format!("{:.1}%", analysis.avg_matchpoint_pct).normal()
            };
            println!("Avg matchpoints:  {}", mp_color);

            // Role-based matchpoint percentages
            if let Some(pct) = analysis.declaring_mp_pct {
                let color = if pct >= 55.0 {
                    format!("{:.1}%", pct).green()
                } else if pct <= 45.0 {
                    format!("{:.1}%", pct).red()
                } else {
                    format!("{:.1}%", pct).normal()
                };
                println!("  Declaring:      {}", color);
            }
            if let Some(pct) = analysis.dummy_mp_pct {
                let color = if pct >= 55.0 {
                    format!("{:.1}%", pct).green()
                } else if pct <= 45.0 {
                    format!("{:.1}%", pct).red()
                } else {
                    format!("{:.1}%", pct).normal()
                };
                println!("  Dummy:          {}", color);
            }
            if let Some(pct) = analysis.defending_mp_pct {
                let color = if pct >= 55.0 {
                    format!("{:.1}%", pct).green()
                } else if pct <= 45.0 {
                    format!("{:.1}%", pct).red()
                } else {
                    format!("{:.1}%", pct).normal()
                };
                println!("  Defending:      {}", color);
            }

            if let Some(avg) = analysis.avg_declarer_vs_field {
                let decl_color = if avg > 0.1 {
                    format!("{:+.2} tricks", avg).green()
                } else if avg < -0.1 {
                    format!("{:+.2} tricks", avg).red()
                } else {
                    format!("{:+.2} tricks", avg).normal()
                };
                println!("Declarer vs field: {}", decl_color);
            }
            println!("Field contract:   {:.1}%", analysis.field_contract_pct);
            println!();

            // Board-by-board results
            println!("{}", "Board-by-Board Results".cyan().bold());
            println!("{}", "----------------------".cyan());
            println!(
                "{:>5} {:>7} {:>8} {:>7} {:>7} {:>10} {:>8} {:>8} {}",
                "Board".bold(),
                "Dir".bold(),
                "Contract".bold(),
                "Score".bold(),
                "MP%".bold(),
                "Field".bold(),
                "Decl".bold(),
                "Cause".bold(),
                "Notes".bold()
            );
            println!(
                "{:->5} {:->7} {:->8} {:->7} {:->7} {:->10} {:->8} {:->8} {:->15}",
                "", "", "", "", "", "", "", "", ""
            );

            for br in &analysis.board_results {
                let dir = match br.direction {
                    PartnershipDirection::NorthSouth => "NS",
                    PartnershipDirection::EastWest => "EW",
                };

                // Format field contract with proper padding before coloring
                let field_contract = br
                    .field_contract
                    .as_ref()
                    .map(|c| {
                        let display = c.display();
                        if br.matched_field_contract {
                            let padded = format!("{:>10}", format!("={}", display));
                            padded.green().to_string()
                        } else {
                            format!("{:>10}", display)
                        }
                    })
                    .unwrap_or_else(|| format!("{:>10}", "-"));

                // Format declarer vs field with proper padding before coloring
                let decl_str = if br.was_declarer {
                    br.declarer_vs_field
                        .map(|d| {
                            let s = format!("{:>8}", format!("{:+.1}", d));
                            if d > 0.1 {
                                s.green().to_string()
                            } else if d < -0.1 {
                                s.red().to_string()
                            } else {
                                s
                            }
                        })
                        .unwrap_or_else(|| format!("{:>8}", "*"))
                } else {
                    format!("{:>8}", "-").dimmed().to_string()
                };

                // Create board number with hyperlink to BBO hand viewer
                let board_display = if let Some(board_data) = data.boards.get(&br.board_number) {
                    // Find the original BoardResult to get player names and contract
                    let board_result = data.results.iter().find(|r| {
                        r.board_number == br.board_number
                            && (r.ns_pair.contains(&analysis.player)
                                || r.ew_pair.contains(&analysis.player))
                    });

                    let seat_players = board_result
                        .map(|r| SeatPlayers::from_partnerships(&r.ns_pair, &r.ew_pair));

                    // Build contract result for URL
                    let contract_result = board_result.and_then(|r| {
                        r.contract.as_ref().map(|c| ContractResult {
                            contract: c.clone(),
                            declarer: r.declarer_direction,
                        })
                    });

                    if let Some(url) = board_data
                        .bbo_handviewer_url(seat_players.as_ref(), contract_result.as_ref())
                    {
                        hyperlink(&url, &format!("{:>5}", br.board_number))
                    } else {
                        format!("{:>5}", br.board_number)
                    }
                } else {
                    format!("{:>5}", br.board_number)
                };

                let cause_str = format_cause(br.cause);

                println!(
                    "{} {:>7} {:>8} {} {} {} {} {} {}",
                    board_display,
                    dir,
                    br.result_str,
                    color_score(br.player_score),
                    color_mp(br.matchpoint_pct),
                    field_contract,
                    decl_str,
                    cause_str,
                    br.notes
                );
            }

            println!();
            println!(
                "{}",
                "Legend: Dir=Direction, MP%=Matchpoint%, Field=Field contract (= if matched)"
                    .dimmed()
            );
            println!(
                "{}",
                "        Decl=Declarer tricks vs field avg (* = only declarer in strain)".dimmed()
            );
            println!(
                "{}",
                "        Cause: Good=skill, Lucky=opp error, Play=declarer, Defense=defense, Auction=bidding, Unlucky=bad luck".dimmed()
            );
            println!(
                "{}",
                "        Click board numbers to view hand in BBO Hand Viewer".dimmed()
            );
        }
    }

    Ok(())
}

fn cmd_board(data: &GameData, board_number: u32, event_id: Option<&str>) -> Result<()> {
    let analysis = analyze_board(data, board_number);

    match analysis {
        None => {
            println!("{}", format!("Board {} not found.", board_number).red());
            return Ok(());
        }
        Some(analysis) => {
            println!(
                "{} {}",
                "Board Analysis:".cyan().bold(),
                format!("Board {}", analysis.board_number).white().bold()
            );
            println!("{}", "=".repeat(50).cyan());

            // Show event link if event_id provided
            if let Some(eid) = event_id {
                let url = board_url(eid, board_number);
                println!("{}", hyperlink(&url, "View board on ACBL").cyan());
            }

            if let Some(field) = &analysis.field_contract {
                println!("Field contract: {}", field.display().white().bold());
            }
            println!("Board type:     {}", analysis.board_type);
            println!();

            // Header
            println!(
                " {:<24} {:>8}  {:>7}  {:>7}  {:>8}  {}",
                "Pair".bold(),
                "Contract".bold(),
                "Score".bold(),
                "MP%".bold(),
                "Cause".bold(),
                "Notes".bold(),
            );
            println!(
                " {:-<24} {:->8}  {:->7}  {:->7}  {:->8}  {:->20}",
                "", "", "", "", "", ""
            );

            for tr in &analysis.results {
                let ns_score = tr.ns_score;
                let ew_score = -tr.ns_score;

                // Determine which side declared for display
                let declaring_ns =
                    matches!(tr.declarer_direction, Direction::North | Direction::South);

                // NS line (first): show contract if NS declared
                let ns_contract = if declaring_ns {
                    format!("{:>8}", tr.result_str)
                } else {
                    format!("{:>8}", "")
                };

                let ns_cause = format_cause(tr.ns_analysis.cause);
                let ns_pair_name = truncate(&tr.ns_pair.display_name(), 24);

                println!(
                    " {:<24} {}  {}  {}  {}  {}",
                    ns_pair_name,
                    ns_contract,
                    color_score(ns_score),
                    color_mp(tr.ns_analysis.matchpoint_pct),
                    ns_cause,
                    tr.ns_analysis.notes,
                );

                // EW line (second): show contract if EW declared
                let ew_contract = if !declaring_ns {
                    format!("{:>8}", tr.result_str)
                } else {
                    format!("{:>8}", "")
                };

                let ew_cause = format_cause(tr.ew_analysis.cause);
                let ew_pair_name = truncate(&tr.ew_pair.display_name(), 24);

                println!(
                    " {:<24} {}  {}  {}  {}  {}",
                    ew_pair_name,
                    ew_contract,
                    color_score(ew_score),
                    color_mp(tr.ew_analysis.matchpoint_pct),
                    ew_cause,
                    tr.ew_analysis.notes,
                );

                // Separator between table results
                println!();
            }

            println!(
                "{}",
                "Cause: Good=skill, Lucky=opp error, Play=declarer, Defense=defense, Auction=bidding, Unlucky=bad luck".dimmed()
            );
        }
    }

    Ok(())
}

/// Format a cause with color
fn format_cause(cause: ResultCause) -> String {
    match cause {
        ResultCause::Good => format!("{:>8}", "Good").green().to_string(),
        ResultCause::Lucky => format!("{:>8}", "Lucky").cyan().to_string(),
        ResultCause::Play => format!("{:>8}", "Play").yellow().to_string(),
        ResultCause::Defense => format!("{:>8}", "Defense").yellow().to_string(),
        ResultCause::Auction => format!("{:>8}", "Auction").magenta().to_string(),
        ResultCause::Unlucky => format!("{:>8}", "Unlucky").red().to_string(),
    }
}

fn cmd_declarer(data: &GameData, player_filter: Option<&str>) -> Result<()> {
    let performances = analyze_declarer_performance(data);

    println!("{}", "Declarer Performance Analysis".cyan().bold());
    println!("{}\n", "=".repeat(29).cyan());

    let filtered: Vec<_> = if let Some(name) = player_filter {
        let normalized = normalize_name(name);
        performances
            .into_iter()
            .filter(|p| p.player.canonical_name.contains(&normalized))
            .collect()
    } else {
        performances
    };

    if filtered.is_empty() {
        println!("{}", "No matching players found.".yellow());
        return Ok(());
    }

    println!(
        "{:<25} {:>8} {:>12}",
        "Player".bold(),
        "Boards".bold(),
        "vs Field".bold()
    );
    println!("{:-<25} {:->8} {:->12}", "", "", "");

    for perf in &filtered {
        let vs_field_val = perf.avg_tricks_vs_field;
        let vs_field = format!("{:+.2}", vs_field_val);
        let vs_field_colored = if vs_field_val > 0.1 {
            vs_field.green()
        } else if vs_field_val < -0.1 {
            vs_field.red()
        } else {
            vs_field.normal()
        };

        println!(
            "{:<25} {:>8} {:>12}",
            truncate(&perf.player.display_name(), 25),
            perf.boards_declared,
            vs_field_colored
        );
    }

    // Show detailed breakdown for single player or small list
    if filtered.len() <= 3 {
        for perf in &filtered {
            println!(
                "\n{} {}",
                perf.player.display_name().white().bold(),
                "- Breakdown by Strain:".cyan()
            );
            println!(
                "{:<10} {:>8} {:>12}",
                "Strain".bold(),
                "Boards".bold(),
                "vs Field".bold()
            );
            println!("{:-<10} {:->8} {:->12}", "", "", "");

            let mut strains: Vec<_> = perf.by_strain.iter().collect();
            strains.sort_by_key(|(s, _)| strain_order(s));

            for (strain_key, sp) in strains {
                let name = strain_display_name(strain_key);
                let vs_field_val = sp.avg_tricks_vs_field;
                let vs_field = format!("{:+.2}", vs_field_val);
                let vs_field_colored = if vs_field_val > 0.1 {
                    vs_field.green()
                } else if vs_field_val < -0.1 {
                    vs_field.red()
                } else {
                    vs_field.normal()
                };
                println!("{:<10} {:>8} {:>12}", name, sp.boards, vs_field_colored);
            }
        }
    }

    Ok(())
}

fn cmd_bidding(data: &GameData, partnership_filter: Option<&str>) -> Result<()> {
    let performances = analyze_bidding_performance(data);

    println!("{}", "Partnership Bidding Analysis".cyan().bold());
    println!("{}\n", "=".repeat(28).cyan());

    let filtered: Vec<_> = if let Some(names) = partnership_filter {
        let parts: Vec<_> = names.split(',').map(|s| normalize_name(s.trim())).collect();
        performances
            .into_iter()
            .filter(|p| {
                parts.iter().any(|name| {
                    p.partnership.player1.canonical_name.contains(name)
                        || p.partnership.player2.canonical_name.contains(name)
                })
            })
            .collect()
    } else {
        performances
    };

    if filtered.is_empty() {
        println!("{}", "No matching partnerships found.".yellow());
        return Ok(());
    }

    println!(
        "{:<40} {:>8} {:>10} {:>10}",
        "Partnership".bold(),
        "Boards".bold(),
        "Field %".bold(),
        "Par %".bold()
    );
    println!("{:-<40} {:->8} {:->10} {:->10}", "", "", "", "");

    for perf in &filtered {
        let par_pct = perf
            .par_strain_accuracy
            .map(|p| format!("{:.1}%", p))
            .unwrap_or_else(|| "N/A".to_string());

        let field_pct = perf.field_agreement;
        let field_str = format!("{:.1}%", field_pct);
        let field_colored = if field_pct >= 70.0 {
            field_str.green()
        } else if field_pct <= 50.0 {
            field_str.red()
        } else {
            field_str.normal()
        };

        println!(
            "{:<40} {:>8} {:>10} {:>10}",
            truncate(&perf.partnership.display_name(), 40),
            perf.boards_analyzed,
            field_colored,
            par_pct
        );
    }

    Ok(())
}

fn cmd_report(data: &GameData, event_id: Option<&str>) -> Result<()> {
    println!("{}", "Bridge Club Game Analysis Report".cyan().bold());
    println!("{}", "=".repeat(32).cyan());

    // Show event link if event_id provided
    if let Some(eid) = event_id {
        let url = event_url(eid);
        println!("{}", hyperlink(&url, "View event results on ACBL").cyan());
    }
    println!();

    // Summary
    println!("{}", "Summary".cyan().bold());
    println!("{}", "-------".cyan());
    println!("Boards: {}", data.boards.len().to_string().white());
    println!("Players: {}", data.players.len().to_string().white());
    println!(
        "Partnerships: {}",
        data.partnerships().len().to_string().white()
    );
    println!("Results: {}", data.results.len().to_string().white());
    println!();

    // Top declarers
    println!("{}", "Top Declarers (by tricks vs field)".cyan().bold());
    println!("{}", "-----------------------------------".cyan());
    let declarers = analyze_declarer_performance(data);
    for (i, perf) in declarers.iter().take(10).enumerate() {
        let vs_field = perf.avg_tricks_vs_field;
        let vs_str = format!("{:+.2}", vs_field);
        let vs_colored = if vs_field > 0.1 {
            vs_str.green()
        } else if vs_field < -0.1 {
            vs_str.red()
        } else {
            vs_str.normal()
        };
        println!(
            "{}. {} ({} boards): {} tricks vs field",
            format!("{:>2}", i + 1).yellow(),
            perf.player.display_name(),
            perf.boards_declared,
            vs_colored
        );
    }
    println!();

    // Top partnerships by field agreement
    println!("{}", "Top Partnerships (by field agreement)".cyan().bold());
    println!("{}", "-------------------------------------".cyan());
    let partnerships = analyze_bidding_performance(data);
    for (i, perf) in partnerships.iter().take(10).enumerate() {
        let par_info = perf
            .par_strain_accuracy
            .map(|p| format!(", {:.1}% par", p))
            .unwrap_or_default();
        println!(
            "{}. {} ({} boards): {:.1}% field{}",
            format!("{:>2}", i + 1).yellow(),
            perf.partnership.display_name(),
            perf.boards_analyzed,
            perf.field_agreement,
            par_info.dimmed()
        );
    }

    Ok(())
}

fn cmd_players(data: &GameData) -> Result<()> {
    println!("{}", "Players in Game".cyan().bold());
    println!("{}\n", "=".repeat(15).cyan());

    let mut players: Vec<_> = data.players.all_players().collect();
    players.sort_by_key(|a| a.display_name());

    for player in players {
        println!("{}", player.display_name());
    }

    println!(
        "\n{} {}",
        "Total:".bold(),
        format!("{} players", data.players.len()).white()
    );
    Ok(())
}

fn cmd_partnerships(data: &GameData) -> Result<()> {
    println!("{}", "Partnerships in Game".cyan().bold());
    println!("{}\n", "=".repeat(20).cyan());

    let mut partnerships = data.partnerships();
    partnerships.sort_by_key(|a| a.display_name());

    for partnership in &partnerships {
        println!("{}", partnership.display_name());
    }

    println!(
        "\n{} {}",
        "Total:".bold(),
        format!("{} partnerships", partnerships.len()).white()
    );
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Convert strain key to display name
fn strain_display_name(strain_key: &str) -> &'static str {
    match strain_key {
        "C" => "Clubs",
        "D" => "Diamonds",
        "H" => "Hearts",
        "S" => "Spades",
        "NT" => "No Trump",
        _ => "Unknown",
    }
}

/// Get sort order for strain keys
fn strain_order(strain_key: &str) -> u8 {
    match strain_key {
        "C" => 0,
        "D" => 1,
        "H" => 2,
        "S" => 3,
        "NT" => 4,
        _ => 5,
    }
}
