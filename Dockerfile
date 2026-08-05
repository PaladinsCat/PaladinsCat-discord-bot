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
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/paladinscat-discord-bot .

EXPOSE 3020
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -sf http://localhost:3020/health || exit 1
ENV RUST_LOG=info HEALTH_PORT=3020
ENTRYPOINT ["./paladinscat-discord-bot"]
