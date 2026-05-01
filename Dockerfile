# syntax=docker/dockerfile:1.7

# ---- builder ----
FROM rust:1-bookworm AS builder
WORKDIR /workspace

# Service workspace goes in bridge-analysis/ so siblings could be COPY'd
# alongside via buildx --build-context (../<sibling>) if we ever need
# hot-edits on bridge-parsers / bridge-types / bridge-encodings. Today we
# build against the git URLs declared in Cargo.toml — the [patch] overrides
# in .cargo/config.toml are local-Mac-only and excluded via .dockerignore.

# Cache prime: copy every workspace member's manifest, stub each member's
# src/, run a dummy build to populate target/. When only service code
# changes, this layer stays cached and the dummy build doesn't re-run.
COPY Cargo.toml Cargo.lock ./bridge-analysis/
COPY parse-files/Cargo.toml ./bridge-analysis/parse-files/Cargo.toml
COPY web/Cargo.toml ./bridge-analysis/web/Cargo.toml
COPY web/build.rs ./bridge-analysis/web/build.rs
WORKDIR /workspace/bridge-analysis
RUN mkdir -p parse-files/src web/src web/static && \
    echo '' > parse-files/src/lib.rs && \
    echo 'fn main() {}' > web/src/main.rs && \
    cargo build --release -p bridge-analysis-web && \
    cargo clean --release -p parse-files -p bridge-analysis-web && \
    rm -rf parse-files/src web/src web/static

# Real source for every member, then the actual build. Service-only edits
# leave the cache-prime layer above untouched. `cargo clean -p` above
# invalidated the workspace crates' fingerprints + artifacts but kept all
# transitive crates.io deps compiled, so this build only re-compiles
# parse-files and bridge-analysis-web.
COPY parse-files/src ./parse-files/src
COPY web/src ./web/src
COPY web/static ./web/static
RUN cargo build --release -p bridge-analysis-web

# ---- runtime ----
FROM debian:bookworm-slim

# mdbtools is required at runtime — bridge-parsers shells out to mdb-export
# and friends to read .BWS (Microsoft Access) files. Without it, BWS+PBN
# uploads fail at parse time.
ARG RUNTIME_PACKAGES="mdbtools"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget ${RUNTIME_PACKAGES} \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 -m service \
    && mkdir -p /data/uploads /data/logs && chown -R service:service /data

USER service
WORKDIR /app

COPY --from=builder /workspace/bridge-analysis/target/release/bridge-analysis-web /app/bridge-analysis-web

ENV PORT=3001
ENV HOST=0.0.0.0
ENV UPLOAD_DIR=/data/uploads
ENV LOG_DIR=/data/logs
ENV LOG_FORMAT=json
ENV LOG_LEVEL=info
EXPOSE 3001

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD wget -q --spider http://localhost:3001/healthz || exit 1

CMD ["/app/bridge-analysis-web"]
