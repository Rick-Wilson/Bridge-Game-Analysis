# CLAUDE.md

This file provides guidance to Claude Code when working with this repository.

## Project Overview

Club Game Analysis — a tool for bridge players to understand their game results
board-by-board, categorizing each result by cause (good play, lucky, auction
error, play error, defense error, unlucky). Available as both a CLI and a web
application, with two ways to get data into it:

1. **Upload BWS + PBN files** (the original flow, for ACBLscore/ACBLlive club
   games — drag and drop on the home page).
2. **Browser-extension push** (for ACBL Live tournament data — the
   [bridge-data-extension](../acbl-live-fetch) scrapes the results page and
   hands a normalized JSON document off to the `/analyze` route).

### One analysis path, multiple input adapters

The analyzer's data model is a single common pipeline. Both inputs feed the
same downstream analysis:

```
                                           ┌─ data/adapters/pbn_bws.rs ─┐
BWS + PBN files ──────────────────────────►│                            │
                                           ├─ data/builder.rs ──────────► Vec<SessionData>
JSON push (acbl-live-fetch extension) ────►│                            │   (one per session)
                                           └─ data/schema.rs (validates)┘
                                                                            ↓
                                                                         analysis (metrics/)
```

The boundary is the **normalized JSON schema** documented in
[`acbl-live-fetch/docs/normalized-schema.md`](../acbl-live-fetch/docs/normalized-schema.md).
Adapters convert their native format into that schema; everything downstream
reads only the schema and is unaware of input format. To add a third source
(BBO LIN, hand-typed data, ...) write a new adapter that emits the schema.

### Workspace Crates

| Crate | Binary | Purpose |
|-------|--------|---------|
| `analysis/` | (library) | Core engine. Schema types in `data/schema.rs`, JSON→GameData in `data/builder.rs`, BWS+PBN→JSON in `data/adapters/pbn_bws.rs`, analysis in `metrics/` |
| `cli/` | `bridge-analysis` | Command-line interface (`ba` alias on dev Mac) |
| `web/` | `bridge-analysis-web` | Axum web server with file upload, JSON ingest, analysis API, admin dashboard |

### Key Dependencies

- `bridge-parsers` (git) — BWS/PBN file parsing (used by the pbn_bws adapter only)
- `bridge-types` (git, via bridge-parsers) — Direction, Strain, Deal, Vulnerability types
- `axum 0.7` — Web framework (web crate)
- `reqwest` — HTTP client for BBA proxy (web crate)
- `serde_json`, `chrono` — JSON ingest + emit (analysis crate)

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
# 1. Download artifact from GitHub Actions
gh run download <RUN_ID> --repo Rick-Wilson/Bridge-Club-Game-Analysis -n bridge-analysis-linux-x64

# 2. Copy to droplet — use a staging filename, the running binary is locked.
scp bridge-analysis-web root@146.190.135.172:/opt/bridge-analysis/bridge-analysis-web.new

# 3. Stop service, swap binary, start. Doing it in one ssh call keeps the
#    service down for ~2 seconds.
ssh root@146.190.135.172 'set -e
  systemctl stop bridge-analysis-web
  mv /opt/bridge-analysis/bridge-analysis-web.new /opt/bridge-analysis/bridge-analysis-web
  chmod +x /opt/bridge-analysis/bridge-analysis-web
  systemctl start bridge-analysis-web'
```

A direct `scp` over the running binary fails (`dest open: Failure`) because
systemd holds the file open — that's why we use the staging-filename pattern.

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
| `GET /` | Serve main SPA page (file-upload entry) |
| `GET /analyze` | Serve same SPA, but bootstrap reads JSON from `sessionStorage["pending-session"]` (extension entry) |
| `POST /api/upload` | Upload BWS+PBN files, returns session ID + session metadata + player/board lists |
| `POST /api/upload-normalized` | Accept normalized JSON in body (extension's POST target). Validates `schema_version` (rejects unknown major versions with 422) |
| `GET /api/sessions?session=...` | List all sessions in an upload (BWS=1, JSON ingest may be many) |
| `GET /api/players?session=...&session_idx=...` | Player list for a session |
| `GET /api/boards?session=...&session_idx=...` | Board-number list for a session |
| `GET /api/player?session=...&session_idx=...&name=...` | Player analysis JSON |
| `GET /api/board?session=...&session_idx=...&num=...` | Board analysis JSON with per-row BBO URLs |
| `POST /api/names?session=...` | Apply ACBL-number → name overrides (session-wide) |
| `POST /api/bba-proxy` | Proxy requests to BBA server (avoids CORS) |
| `GET /health` | Health check |
| `GET /admin/dashboard?key=...` | Admin analytics dashboard |
| `GET /admin/api/stats?key=...` | Usage statistics JSON |

`session_idx` defaults to `0` and is omitted from URLs when there's only one
session (the BWS+PBN case). Multi-session uploads (typical for tournament
data from the extension) get a session-selector dropdown in the UI.

### Extension handoff protocol (`/analyze`)

The browser extension at [acbl-live-fetch](../acbl-live-fetch) performs the
following sequence to hand off a freshly-scraped tournament:

1. Build the normalized JSON document in the extension.
2. Open a new tab to `https://club-game-analysis.bridge-classroom.com/analyze`.
3. Have a content script in that tab write the JSON to
   `window.sessionStorage["pending-session"]`. Because content-script timing
   is not guaranteed relative to page scripts, the SPA polls sessionStorage
   for ~2s and also listens for a `bridge-classroom-handoff` `window` event
   the extension can dispatch to short-circuit the wait.

The SPA's `/analyze` bootstrap has three explicit states:

- **DATA**: pending-session present, valid, accepted → normal player/board UI.
- **EMPTY**: no pending-session → friendly card with link back to `/`.
- **ERROR**: malformed JSON, unsupported `schema_version`, or server reject →
  error card with the specific message and link back to `/`.

The fragment `#sid={uuid}` is for extension-side bookkeeping only; the SPA
ignores it. Reads of `pending-session` are one-shot — consumed via
`removeItem` immediately so a refresh shows EMPTY rather than re-applying
stale data.

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
| [Bridge-Parsers](../Bridge-Parsers) | PBN/LIN/BWS file parsing | upstream dependency (used only by pbn_bws adapter) |
| [bridge-solver](../bridge-solver) | Double-dummy solver | upstream dependency |
| [acbl-live-fetch](../acbl-live-fetch) | Browser extension that scrapes ACBL Live tournament results and hands off normalized JSON to `/analyze` | **input source** (the JSON ingest path) — owns `docs/normalized-schema.md`, the contract between adapters and the analyzer |
| [bridge-wrangler](../bridge-wrangler) | CLI tool | sibling |
| [bridge-classroom](../bridge-classroom) | Main website | parent site (footer, landing page) |
| [BBA-CLI](../BBA-CLI) | BBA auction engine | BBA proxy target, deployment model reference |

### Schema co-evolution

The normalized JSON schema is owned by [`acbl-live-fetch/docs/normalized-schema.md`](../acbl-live-fetch/docs/normalized-schema.md).
When the schema changes, both repos must move together:

- analyzer: update `analysis/src/data/schema.rs` (serde types) and
  `analysis/src/data/builder.rs` (schema → GameData conversion). Bump
  `SUPPORTED_MAJOR` only on a breaking change.
- extension: update its emitter and the doc's worked example.
- analyzer's BWS adapter (`analysis/src/data/adapters/pbn_bws.rs`) emits the
  same schema; update it too if the change affects fields the adapter writes.

Validate end-to-end with `cargo test` (analyzer) + `npm test` (extension). The
analyzer enforces `schema_version` major-version compatibility — unknown major
versions return 422 from `/api/upload-normalized`.

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
