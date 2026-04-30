SERVICE := "bridge-analysis"
# ghcr.io paths must be lowercase even though the GitHub user is Rick-Wilson.
IMAGE   := "ghcr.io/rick-wilson/" + SERVICE
DROPLET := "bridge-droplet"

# Sibling crate path-deps. Empty today — we build against the git URLs
# declared in Cargo.toml. To add hot-edit support for a sibling (e.g.
# bridge-parsers), add `--build-context bridge-parsers=../Bridge-Parsers`
# here AND a matching COPY line in the Dockerfile AND a [patch] entry
# (currently the local-only one in .cargo/config.toml would need to move
# into Cargo.toml so it's visible to the container build too).
SIBLING_CONTEXTS := ""

default:
    @just --list

# Ensure colima is running (no-op if already up).
_colima-up:
    @colima status >/dev/null 2>&1 || (echo "Starting colima..." && colima start)

# Native-arch build (fast, for local testing).
build: _colima-up
    docker build {{SIBLING_CONTEXTS}} -t {{IMAGE}}:local .

# Run locally on port 3001, mounting ./data for persistent state.
run: build
    docker run --rm -p 3001:3001 \
        -e DASHBOARD_SECRET=devsecret \
        -e LOG_FORMAT=pretty \
        -v {{justfile_directory()}}/data:/data \
        {{IMAGE}}:local

# Run cargo locally without docker (for fastest iteration).
dev:
    cargo run -p bridge-analysis-web

# Cross-arch build for the droplet (linux/amd64).
#
# Forced through the `colima` builder (host daemon, uses Apple Rosetta on
# Apple Silicon) instead of whatever buildx's active builder is. The
# default `docker-container` driver carries its own QEMU emulator inside
# the buildkit container, which segfaults running rustc x86_64. The
# `colima` builder uses the colima VM directly, where Rosetta-via-VZ
# emulates amd64 at near-native speed and rustc runs cleanly.
#
# Prereq: `colima start --vz-rosetta` (already done if you've run this
# successfully once; persists across reboots).
build-prod: _colima-up
    docker buildx --builder colima build {{SIBLING_CONTEXTS}} --platform linux/amd64 -t {{IMAGE}}:dev --load .

# Push the dev image to ghcr.io.
push: build-prod
    docker push {{IMAGE}}:dev

# Deploy the dev image to the droplet.
deploy: push
    ssh {{DROPLET}} '/opt/bridge-craftwork/scripts/deploy.sh {{SERVICE}}'

# Tag and push a release. CI will build and push the versioned image.
release VERSION:
    git tag {{VERSION}}
    git push origin {{VERSION}}
    @echo "GitHub Actions will build {{VERSION}}. Once CI is green:"
    @echo "  just deploy-version {{VERSION}}"

# Promote a tagged version on the droplet.
deploy-version VERSION:
    ssh {{DROPLET}} 'sed -i "s/^BRIDGE_ANALYSIS_TAG=.*/BRIDGE_ANALYSIS_TAG={{VERSION}}/" /opt/bridge-craftwork/.env && \
        /opt/bridge-craftwork/scripts/deploy.sh {{SERVICE}}'

# Tail logs from the droplet.
logs:
    ssh {{DROPLET}} 'cd /opt/bridge-craftwork && docker compose logs -f --tail 100 {{SERVICE}}'

# Shell into the running container.
shell:
    ssh -t {{DROPLET}} 'cd /opt/bridge-craftwork && docker compose exec {{SERVICE}} /bin/sh'

# Run all checks the way CI does.
check:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    cargo test --all
