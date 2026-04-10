# CLAUDE.md

This file provides guidance to Claude Code when working with this repository.

## Project Overview

Club Game Analysis — a tool for bridge players to understand their club game results board-by-board, categorizing each result by cause (good play, lucky, auction error, play error, defense error, unlucky). Available as both a CLI and a web application.

### Workspace Crates

| Crate | Binary | Purpose |
|-------|--------|---------|
| `analysis/` | (library) | Core analysis engine: data loading, matchpoints, cause analysis, board classification |
| `cli/` | `bridge-analysis` | Command-line interface (`ba` alias on dev Mac) |
| `web/` | `bridge-analysis-web` | Axum web server with file upload, analysis API, admin dashboard |

### Key Dependencies

- `bridge-parsers` (local path `../../Bridge-Parsers`) — BWS/PBN file parsing, hand records
- `bridge-types` (git, via bridge-parsers) — Direction, Strain, Deal, Vulnerability types
- `axum 0.7` — Web framework (web crate)
- `reqwest` — HTTP client for BBA proxy (web crate)

## Build & Test Commands

```bash
cargo build                              # Build all crates (debug)
cargo build --release                    # Build all crates (release)
cargo build -p bridge-analysis-web       # Build web server only
cargo test                               # Run all tests
cargo clippy --all-targets -- -D warnings # Lint (treat warnings as errors)
cargo fmt --all                          # Format all code
```

### Local Testing

```bash
# Run web server locally (serves at http://localhost:3001/)
cargo run -p bridge-analysis-web

# Run CLI
cargo run -p bridge-analysis -- player --bws FILE.BWS --pbn FILE.pbn --name "Player Name"
cargo run -p bridge-analysis -- board --bws FILE.BWS --pbn FILE.pbn --board 1
```

The web server reads `BASE_PATH` env var to nest under a path prefix (default: empty = root).
For local dev, just run it and access `http://localhost:3001/`.

**Important:** The web crate embeds static files (HTML/JS/CSS) at compile time via `include_str!`/`include_bytes!`. A `build.rs` watches the `static/` directory, but if static changes aren't picked up, `touch web/src/api.rs` forces a recompile.

## Pre-commit Requirements

Before committing, always run and fix:
1. `cargo fmt --all` - Format all code
2. `cargo clippy --all-targets -- -D warnings` - Fix all clippy warnings
3. `cargo test` - Ensure all tests pass

## Code Standards

- No `unwrap()` or `expect()` outside test code - use proper error handling
- No `println!()` in library code (CLI and web binaries are OK)
- All public functions must have doc comments (`///`)
- All `unsafe` blocks must have a comment explaining why they're safe
- Prefer editing existing files over creating new ones

## Production Deployment

### Architecture

```
User → Cloudflare DNS → DigitalOcean Droplet → Caddy → bridge-analysis-web
```

- **Domain:** `club-game-analysis.bridge-classroom.com`
- **DNS:** Cloudflare A record → droplet IP (DNS only, not proxied — Caddy handles TLS)
- **Reverse proxy:** Caddy at `/opt/livekit/Caddyfile`
- **Droplet IP:** `146.190.135.172`
- **SSH:** `ssh root@146.190.135.172` (Mac id_ed25519 key)
- **Service:** systemd `bridge-analysis-web`
- **Install path:** `/opt/bridge-analysis/`
- **Port:** 3001
- **Requires:** `mdbtools` installed on server (for BWS file parsing)

### Caddy Configuration

The Caddyfile at `/opt/livekit/Caddyfile` includes:
```
club-game-analysis.bridge-classroom.com {
    reverse_proxy localhost:3001
}
```

**Restart Caddy after changes:**
```bash
ssh root@146.190.135.172 'cd /opt/livekit && docker compose restart caddy'
```

### CI/CD Pipeline

GitHub Actions (`.github/workflows/build.yml`) builds on push to `main` or version tags:
1. Checks out this repo and Bridge-Parsers side by side
2. Installs mdbtools
3. Builds both `bridge-analysis` (CLI) and `bridge-analysis-web` (server) for Linux x64
4. Runs tests
5. Uploads artifacts
6. On version tags (`v*`): creates a GitHub Release with tarball

### Deploy New Version

After CI builds successfully:
```bash
# Download artifact from GitHub Actions
gh run download <RUN_ID> --repo Rick-Wilson/Bridge-Club-Game-Analysis -n bridge-analysis-linux-x64

# Copy to droplet
scp bridge-analysis-web root@146.190.135.172:/opt/bridge-analysis/

# Restart service
ssh root@146.190.135.172 'systemctl restart bridge-analysis-web'
```

### Server Management

```bash
# Check status
ssh root@146.190.135.172 'systemctl status bridge-analysis-web --no-pager'

# View logs
ssh root@146.190.135.172 'journalctl -u bridge-analysis-web -n 50 --no-pager'

# Restart
ssh root@146.190.135.172 'systemctl restart bridge-analysis-web'
```

### Environment Configuration

Environment file at `/opt/bridge-analysis/.env`:
```
HOST=0.0.0.0
PORT=3001
LOG_DIR=/opt/bridge-analysis/logs
UPLOAD_DIR=/opt/bridge-analysis/uploads
ADMIN_KEY=<set on server, not in repo>
```

### Other Services on the Same Droplet

| Service | URL | Port | Config |
|---------|-----|------|--------|
| BBA Server | `bba.harmonicsystems.com` | 5000 | `/opt/bba-server/` |
| LiveKit | `livekit.bridge-classroom.com` | 7880 | `/opt/livekit/` |

## Web Application Features

### API Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET /` | Serve main SPA page |
| `POST /api/upload` | Upload BWS+PBN files, returns session ID + player/board lists |
| `GET /api/player?session=...&name=...` | Player analysis JSON |
| `GET /api/board?session=...&num=...` | Board analysis JSON with per-row BBO URLs |
| `POST /api/bba-proxy` | Proxy requests to BBA server (avoids CORS) |
| `GET /health` | Health check |
| `GET /admin/dashboard?key=...` | Admin analytics dashboard |
| `GET /admin/api/stats?key=...` | Usage statistics JSON |

### Analytics

- IP addresses anonymized via SHA-256 → friendly "FirstName_Surname" pseudonyms
- CSV audit logs in `LOG_DIR` with monthly rotation
- Tracks: action, browser, device type, duration
- Admin dashboard shows daily usage, browser/device breakdown, top visitors

### External API Integrations

- **BSOL** (`dds.bridgewebs.com`): Double-dummy analysis for board view (called from browser)
- **BBA** (`bba.harmonicsystems.com`): Sample auction generation (proxied through server to avoid CORS)

## Git Configuration

Use SSH for all GitHub operations:
- Clone/push/pull: `git@github.com:Rick-Wilson/repo.git` (not `https://`)
- Remote URLs should use SSH format

## Related Projects

All located at `/Users/rick/Development/GitHub/`:

| Project | Description | Relationship |
|---------|-------------|--------------|
| [bridge-types](../bridge-types) | Core bridge types | upstream dependency |
| [Bridge-Parsers](../Bridge-Parsers) | PBN/LIN/BWS file parsing | upstream dependency |
| [bridge-solver](../bridge-solver) | Double-dummy solver | upstream dependency |
| [bridge-wrangler](../bridge-wrangler) | CLI tool | sibling |
| [bridge-classroom](../bridge-classroom) | Main website | parent site (footer, landing page) |
| [BBA-CLI](../BBA-CLI) | BBA auction engine | BBA proxy target, deployment model reference |

## Notifications

Send Pushover notifications when work is blocked or completed:

```bash
pushover "message" "title"    # title defaults to "Claude Code"
```

**When to notify:**
- Waiting for user input or permission
- Task completed after extended work
- Build/test failures that need attention
- Any situation where work is paused and user may not notice
