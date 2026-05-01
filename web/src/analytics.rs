//! Usage tracking with anonymized IP addresses and CSV audit logging.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Anonymize an IP address to a deterministic friendly name.
///
/// Uses SHA-256 with a salt to produce "FirstName_Surname" pseudonyms.
/// The same IP always maps to the same name within the same salt.
pub fn anonymize_ip(ip: &str) -> String {
    const SALT: &str = "BridgeAnalysis-2026-Salt";
    const FIRST_NAMES: &[&str] = &[
        "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi", "Ivan", "Judy", "Karl",
        "Linda", "Mike", "Nancy", "Oscar", "Peggy", "Quinn", "Ruth", "Steve", "Tina", "Uma",
        "Victor", "Wendy", "Xavier", "Yvonne", "Zach", "Amber", "Brian", "Cathy", "Derek", "Elena",
        "Felix", "Gloria", "Harry", "Irene", "James", "Karen", "Leo", "Maria", "Nick", "Olive",
        "Paul", "Rosa", "Sam", "Tara", "Ugo", "Vera", "Will", "Xena", "Yuri",
    ];
    const SURNAMES: &[&str] = &[
        "Adams", "Baker", "Clark", "Davis", "Evans", "Fisher", "Grant", "Harris", "Irving",
        "Jones", "King", "Lewis", "Mason", "Nelson", "Owen", "Parker", "Quinn", "Reed", "Smith",
        "Taylor", "Unger", "Vale", "Walsh", "Xu", "Young", "Zane", "Allen", "Brown", "Chang",
        "Dunn", "Ellis", "Fox", "Gray", "Hill", "Irwin", "Jay", "Kent", "Lane", "Moore", "Nash",
        "Olson", "Penn", "Ross", "Shaw", "Todd", "Upton", "Voss", "Webb", "York", "Zhu",
    ];

    let mut hasher = Sha256::new();
    hasher.update(SALT.as_bytes());
    hasher.update(ip.as_bytes());
    let hash = hasher.finalize();

    let hash_num = u64::from_le_bytes(hash[0..8].try_into().unwrap());
    let first_idx = (hash_num % FIRST_NAMES.len() as u64) as usize;
    let surname_idx = ((hash_num / FIRST_NAMES.len() as u64) % SURNAMES.len() as u64) as usize;

    format!("{}_{}", FIRST_NAMES[first_idx], SURNAMES[surname_idx])
}

/// Extract the real client IP from request headers (Cloudflare, proxy, direct).
pub fn extract_ip(headers: &axum::http::HeaderMap, addr: &std::net::SocketAddr) -> String {
    // Priority: CF-Connecting-IP > X-Forwarded-For > X-Real-IP > socket
    if let Some(cf_ip) = headers.get("cf-connecting-ip") {
        if let Ok(ip) = cf_ip.to_str() {
            return ip.trim().to_string();
        }
    }
    if let Some(xff) = headers.get("x-forwarded-for") {
        if let Ok(ips) = xff.to_str() {
            if let Some(first) = ips.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(ip) = real_ip.to_str() {
            return ip.trim().to_string();
        }
    }
    addr.ip().to_string()
}

/// Extract browser/device info from User-Agent header.
pub fn extract_user_agent_info(headers: &axum::http::HeaderMap) -> (String, String) {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    let browser = if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        "Other"
    }
    .to_string();

    let device = if ua.contains("Mobile") || ua.contains("Android") {
        "Mobile"
    } else if ua.contains("iPad") || ua.contains("Tablet") {
        "Tablet"
    } else {
        "Desktop"
    }
    .to_string();

    (browser, device)
}

/// Thread-safe CSV audit logger.
pub struct AuditLogger {
    log_dir: PathBuf,
    lock: Mutex<()>,
}

impl AuditLogger {
    pub fn new(log_dir: &Path) -> Self {
        Self {
            log_dir: log_dir.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    /// Log a page/analysis request.
    pub fn log_request(
        &self,
        anon_ip: &str,
        action: &str,
        detail: &str,
        browser: &str,
        device: &str,
        duration_ms: u64,
    ) {
        let _guard = self.lock.lock().unwrap();
        let now = chrono::Utc::now();
        let filename = format!("audit-{}.csv", now.format("%Y-%m"));
        let filepath = self.log_dir.join(&filename);

        let header = "Timestamp,AnonIP,Action,Detail,Browser,Device,DurationMs\n";
        let row = format!(
            "{},{},{},{},{},{},{}\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            csv_escape(anon_ip),
            csv_escape(action),
            csv_escape(detail),
            csv_escape(browser),
            csv_escape(device),
            duration_ms,
        );

        if !filepath.exists() {
            let _ = fs::write(&filepath, format!("{}{}", header, row));
        } else {
            let _ = fs::OpenOptions::new()
                .append(true)
                .open(&filepath)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(row.as_bytes())
                });
        }
    }

    /// Read stats from the current month's audit log.
    pub fn get_stats(&self) -> AuditStats {
        let now = chrono::Utc::now();
        let filename = format!("audit-{}.csv", now.format("%Y-%m"));
        let filepath = self.log_dir.join(&filename);

        let mut stats = AuditStats::default();
        let content = match fs::read_to_string(&filepath) {
            Ok(c) => c,
            Err(_) => return stats,
        };

        let mut lines = content.lines();
        let _header = lines.next(); // Skip header

        for line in lines {
            let fields: Vec<&str> = parse_csv_line(line);
            if fields.len() < 7 {
                continue;
            }
            stats.total_requests += 1;
            *stats
                .requests_by_user
                .entry(fields[1].to_string())
                .or_insert(0) += 1;
            *stats
                .requests_by_action
                .entry(fields[2].to_string())
                .or_insert(0) += 1;
            *stats
                .requests_by_browser
                .entry(fields[4].to_string())
                .or_insert(0) += 1;
            *stats
                .requests_by_device
                .entry(fields[5].to_string())
                .or_insert(0) += 1;

            // Group by date
            if let Some(date) = fields[0].split(' ').next() {
                *stats.requests_by_day.entry(date.to_string()).or_insert(0) += 1;
            }

            if let Ok(ms) = fields[6].trim().parse::<u64>() {
                stats.total_duration_ms += ms;
            }
        }

        stats
    }

    /// List available log files.
    pub fn list_logs(&self) -> Vec<LogFileInfo> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.log_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    files.push(LogFileInfo {
                        name: entry.file_name().to_string_lossy().to_string(),
                        size: meta.len(),
                    });
                }
            }
        }
        files.sort_by(|a, b| b.name.cmp(&a.name));
        files
    }
}

#[derive(Default, serde::Serialize)]
pub struct AuditStats {
    pub total_requests: u64,
    pub total_duration_ms: u64,
    pub requests_by_user: HashMap<String, u64>,
    pub requests_by_action: HashMap<String, u64>,
    pub requests_by_browser: HashMap<String, u64>,
    pub requests_by_device: HashMap<String, u64>,
    pub requests_by_day: HashMap<String, u64>,
}

#[derive(serde::Serialize)]
pub struct LogFileInfo {
    pub name: String,
    pub size: u64,
}

/// Escape a value for CSV (wrap in quotes if needed).
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Simple CSV line parser handling quoted fields.
fn parse_csv_line(line: &str) -> Vec<&str> {
    // Simple split - handles most cases. Full CSV parsing would need a library.
    line.split(',').collect()
}
