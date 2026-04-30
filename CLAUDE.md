# CLAUDE.md

This file provides guidance to Claude Code when working with this repository.

## Project Overview

Club Game Analysis — a tool for bridge players to understand their game results
board-by-board, categorizing each result by cause (good play, lucky, auction
error, play error, defense error, unlucky). Web app, two input paths:

1. **Upload BWS + PBN files** (the original flow, for ACBLscore/ACBLlive club
   games — drag and drop on the home page).
2. **Browser-extension push** (for ACBL Live tournament data — the
   [bridge-data-extension](../acbl-live-fetch) scrapes the results page and
   hands a normalized JSON document off to the `/analyze` route).

### Architecture: Rust = file translation, JS = analysis

```
┌──────────── server (Rust container) ────────────┐    ┌── browser ──┐
                                                  │    │             │
BWS + PBN  ──► data/adapters/pbn_bws.rs ──┐       │    │  index.html │
                                          ├──► normalized JSON ──► JS analyzer
JSON push ───► data/schema.rs (validate) ─┘   ↑   │    │  (matchpoints,
                                              │   │    │   DVF, cause
                          enrich_tricks  ─────┤   │    │   analysis,
                          enrich_handviewer ──┘   │    │   rendering)
                                                  │    │             │
                                  data.json on disk    │             │
└──────────────────────────────────────────────────┘    └─────────────┘
```

The Rust server's job is the **BWS/PBN → JSON translation**, two schema-walk
enrichment passes (derive missing trick counts from score; replace adapter-
emitted handviewer URLs with canonical BBO LIN URLs that include a
constructed auction), and persistence. Everything else — matchpoint
calculation, declarer-vs-field, cause analysis, all the rendering — runs
client-side in the SPA's inlined JS.

The boundary is the **normalized JSON schema** documented in
[`acbl-live-fetch/docs/normalized-schema.md`](../acbl-live-fetch/docs/normalized-schema.md).
To add a third source (BBO LIN, hand-typed data, ...) write a new adapter
that emits the schema.

### Workspace Crates

| Crate | Binary | Purpose |
|-------|--------|---------|
| `parse-files/` | (library) | Schema types in `data/schema.rs`, BWS+PBN→JSON adapter in `data/adapters/pbn_bws.rs`, schema-walk enrich passes in `data/builder.rs`. |
| `web/` | `bridge-analysis-web` | Axum web server: file upload, JSON ingest, name-overrides, BBA proxy, admin dashboard, static SPA serving. Walks the schema directly via `web/src/upload_helpers.rs` to shape upload responses. |

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
# or:
just dev
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

Migrated 2026-04-30 from a native systemd binary to a container in the [bridge-craftwork-platform](../bridge-craftwork-platform) compose stack. The systemd unit is gone; everything below describes the current container path.

### Architecture

```
User → Cloudflare DNS → edge Caddy (container, host network) → 127.0.0.1:3001 → bridge-analysis container
```

- **Domain:** `club-game-analysis.bridge-classroom.com` (Cloudflare DNS-only — edge Caddy terminates TLS via Let's Encrypt).
- **Edge Caddy:** container in `/opt/edge/` on the droplet (managed by platform repo's `edge/Caddyfile`). Stanza already in place from before the migration; no change needed.
- **Service:** `bridge-analysis` block in `/opt/bridge-craftwork/docker-compose.yml` (file lives in the platform repo at `droplet/docker-compose.yml`, symlinked into `/opt/bridge-craftwork/`).
- **Image:** `ghcr.io/rick-wilson/bridge-analysis:dev` for iteration, `:vX.Y.Z` and `:latest` from CI on tags.
- **Port:** 3001 (loopback only — `127.0.0.1:3001:3001`).
- **Volume:** `/opt/bridge-craftwork/data/services/bridge-analysis/{uploads,logs}/` ↔ container's `/data/`.
- **mdbtools** is in the runtime image (`RUNTIME_PACKAGES="mdbtools"` in [Dockerfile](Dockerfile)). No host install required.
- **SSH:** `ssh bridge-droplet` (a maintainer-local alias in `~/.ssh/config`).

### Local build → droplet (the iteration path)

The [justfile](justfile) wraps the whole local pipeline. Prereq once: `colima start --vz-rosetta` (Apple Silicon needs Rosetta-via-VZ to cross-compile to amd64 cleanly; QEMU segfaults rustc).

```sh
just build      # docker buildx → linux/amd64 → ghcr.io/rick-wilson/bridge-analysis:dev (local)
just push       # build + docker push :dev to ghcr.io
just deploy     # push + ssh bridge-droplet '/opt/bridge-craftwork/scripts/deploy.sh bridge-analysis'
just logs       # tail droplet logs
```

`deploy.sh` is generic: `docker compose pull <service> && docker compose up -d <service>`.

### CI/CD Pipeline

GitHub Actions (`.github/workflows/ci.yml`) on push to `main` and on `v*` tags:
1. `cargo fmt --check && cargo clippy -- -D warnings && cargo test --all`.
2. `docker buildx build --platform linux/amd64` and push to `ghcr.io/rick-wilson/bridge-analysis`:
   - `:main` for branch pushes
   - `:vX.Y.Z` and `:latest` for tags

To promote a tagged release to the droplet: `just release vX.Y.Z` (tags + pushes, then `just deploy-version vX.Y.Z` after CI).

### Server Management

```bash
# Check status
ssh bridge-droplet 'cd /opt/bridge-craftwork && docker compose ps bridge-analysis'

# View logs (tail)
ssh bridge-droplet 'cd /opt/bridge-craftwork && docker compose logs -f --tail 100 bridge-analysis'

# Restart
ssh bridge-droplet 'cd /opt/bridge-craftwork && docker compose restart bridge-analysis'

# Shell into the running container
ssh -t bridge-droplet 'cd /opt/bridge-craftwork && docker compose exec bridge-analysis /bin/sh'
```

### Environment Configuration

Env vars are wired in the platform repo's `droplet/docker-compose.yml`. Secrets come from `/opt/bridge-craftwork/.env` (mode 600, never committed):

```
BRIDGE_ANALYSIS_TAG=dev                 # flip to vX.Y.Z to pin a release
BRIDGE_ANALYSIS_DASHBOARD_SECRET=…      # was ADMIN_KEY in the old systemd .env
```

The compose file injects these plus static values (`PORT=3001`, `HOST=0.0.0.0`, `LOG_LEVEL=info`, `LOG_FORMAT=json`, `UPLOAD_DIR=/data/uploads`, `LOG_DIR=/data/logs`).

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
| `POST /api/upload` | Upload BWS+PBN files, runs the adapter + enrich passes, persists data.json. Response: session ID + session metadata + counts + per-pair ACBL map |
| `POST /api/upload-normalized` | Accept normalized JSON in body (extension's POST target). Validates `schema_version` (rejects unknown major versions with 422), enriches, persists |
| `GET /api/sessions?session=...` | List all sessions in an upload (BWS=1, JSON ingest may be many) |
| `GET /api/normalized?session=...` | Stream the persisted data.json. The browser fetches this once after upload and runs all analysis client-side over it |
| `POST /api/names?session=...` | Apply ACBL-number → name overrides; mutates data.json in place and re-runs the handviewer-URL enrichment |
| `POST /api/bba-proxy` | Proxy requests to BBA server (avoids CORS) |
| `GET /healthz` | Service-contract health (`{status, version, uptime_seconds}`); `/health` 308's here |
| `GET /metrics` | Prometheus text-format metrics |
| `GET /admin/dashboard?key=...` | Admin analytics dashboard |
| `GET /admin/api/stats?key=...` | Usage statistics JSON |

The previously-served `/api/players`, `/api/boards`, `/api/player`, `/api/board`
endpoints (server-side analysis) were removed in 2026-04 when the JS
analyzer became the only path; the SPA computes those lists and per-player /
per-board analysis directly from `cachedNormalized` (the result of one
`/api/normalized` fetch after upload).

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

- server: update `parse-files/src/data/schema.rs` (serde types). Bump
  `SUPPORTED_MAJOR` only on a breaking change.
- server: if the change affects fields `parse-files/src/data/builder.rs`'s
  enrich passes (`enrich_tricks`, `enrich_handviewer_urls`) read or
  write, update those.
- server's BWS adapter (`parse-files/src/data/adapters/pbn_bws.rs`)
  emits the same schema; update it too if the change affects fields
  the adapter writes.
- web's upload helpers (`web/src/upload_helpers.rs`) walk the schema
  for the upload response shape; update if a relevant field changes.
- SPA: the JS analyzer in `web/static/index.html` reads the schema
  directly. Update its derive functions and renderers to match.
- extension: update its emitter and the doc's worked example.

Validate end-to-end with `cargo test --all` (server) + `npm test`
(extension). The server enforces `schema_version` major-version
compatibility — unknown major versions return 422 from
`/api/upload-normalized`.

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
