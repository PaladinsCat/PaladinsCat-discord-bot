# Multi-stage build for the Rust PaladinsCat Discord bot.
# Builds from REPO ROOT context (deploy-script convention). The bot crate lives
# under src/discord-bot-rust; the standalone Dockerfile there builds with that
# dir as context, so this wrapper copies the crate in from the repo root.
FROM rust:slim AS builder

# OpenSSL headers (required by reqwest)
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY src/discord-bot-rust/Cargo.toml .
COPY src/discord-bot-rust/Cargo.lock .
COPY src/discord-bot-rust/src/ src/
COPY src/discord-bot/src/paladins-avatar-assets.ts /discord-bot/src/paladins-avatar-assets.ts
RUN cargo build --release

# Runtime stage — trixie to match builder's GLIBC 2.39
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl chromium fontconfig fonts-inter fonts-dejavu-core && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/paladinscat-discord-bot .
COPY dev/prototypes/ dev/prototypes/
COPY ["src/frontend/public/images/champions/Champion * Icon.avif", "src/frontend/public/images/champions/"]
COPY ["src/frontend/public/images/champions/Talent*.png", "src/frontend/public/images/champions/"]
COPY ["src/frontend/public/images/maps/Match_*.avif", "src/frontend/public/images/maps/"]
COPY src/frontend/public/images/rank-tiers/ src/frontend/public/images/rank-tiers/
COPY src/frontend/public/images/icons/ src/frontend/public/images/icons/

EXPOSE 3020
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -sf http://localhost:3020/health || exit 1
ENV RUST_LOG=info \
    HEALTH_PORT=3020 \
    CHROME_PATH=/usr/bin/chromium \
    PALADINSCAT_RENDER_WEB_URL=http://frontend:3000
ENTRYPOINT ["./paladinscat-discord-bot"]
