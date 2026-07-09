# syntax=docker/dockerfile:1.7

FROM oven/bun:1 AS web-builder

WORKDIR /app
COPY . .
RUN bun install --frozen-lockfile
RUN bun run build:server-web

FROM rust:1-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        libayatana-appindicator3-dev \
        libgtk-3-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
COPY --from=web-builder /app/src-tauri/server-web-dist ./src-tauri/server-web-dist

WORKDIR /app/src-tauri
RUN cargo build --release --bin kam-server --no-default-features --features server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libayatana-appindicator3-1 \
        libgtk-3-0 \
        libwebkit2gtk-4.1-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/kam --shell /usr/sbin/nologin kam \
    && mkdir -p /data \
    && chown -R kam:kam /data /home/kam

COPY --from=builder /app/src-tauri/target/release/kam-server /usr/local/bin/kam-server
COPY --from=web-builder /app/src-tauri/server-web-dist /opt/kam/server-web

ENV KAM_HOST=0.0.0.0 \
    KAM_PORT=8765 \
    KAM_DATA_DIR=/data \
    KAM_WEB_DIR=/opt/kam/server-web \
    RUST_LOG=info

USER kam
VOLUME ["/data"]
EXPOSE 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${KAM_PORT}/healthz" >/dev/null || exit 1

CMD ["kam-server"]
