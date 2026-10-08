# Copyright (C) 2026 Marcos Gabriel Miller
FROM rust:1.95-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /app bot \
    && mkdir -p /app/data \
    && chown bot /app/data
WORKDIR /app
COPY --from=builder /src/target/release/money_manager_telegram /usr/local/bin/money_manager_telegram
USER bot
ENV DATABASE_PATH=/app/data/bot.sqlite
VOLUME ["/app/data"]
CMD ["money_manager_telegram"]
