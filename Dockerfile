FROM rust:1.97-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY legacy-node/src/paladins-avatar-assets.ts ./legacy-node/src/paladins-avatar-assets.ts
COPY assets/templates ./assets/templates
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl chromium fontconfig fonts-inter fonts-dejavu-core fonts-noto-core fonts-noto-cjk \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 1000 --create-home paladinscat
WORKDIR /app
COPY --from=builder /build/target/release/paladinscat-discord-bot /usr/local/bin/paladinscat-discord-bot
COPY assets/templates ./assets/templates
USER 1000:1000
EXPOSE 3020
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD curl -sf http://127.0.0.1:3020/health || exit 1
ENV RUST_LOG=info HEALTH_PORT=3020 CHROME_PATH=/usr/bin/chromium
ENTRYPOINT ["/usr/local/bin/paladinscat-discord-bot"]
