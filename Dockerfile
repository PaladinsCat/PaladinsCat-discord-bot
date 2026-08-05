# Multi-stage Docker build — builds Linux binary from source, ships minimal runtime
FROM rust:slim AS builder

# Install OpenSSL headers (required by reqwest)
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml .
COPY src/ src/
# Don't copy Cargo.lock so we get fresh compatible versions
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/paladinscat-discord-bot .
COPY .env .
COPY docker-compose.yml .
COPY test-all-features.sh .

EXPOSE 3020
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -sf http://localhost:3020/health || exit 1
ENV RUST_LOG=info API_BASE_URL=http://localhost:3001/api HEALTH_PORT=3020
ENTRYPOINT ["./paladinscat-discord-bot"]
