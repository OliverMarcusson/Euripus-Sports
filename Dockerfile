FROM rust:1.88-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY config ./config
COPY tests ./tests
COPY v1.md ./v1.md

RUN cargo build --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates chromium curl \
    && groupadd --gid 10001 sports-api \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /tmp sports-api \
    && install -d -o 10001 -g 10001 /data \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sports-api /usr/local/bin/sports-api
COPY config ./config
COPY tests ./tests
COPY v1.md ./v1.md

ENV HOME=/tmp \
    XDG_CONFIG_HOME=/tmp/.config \
    XDG_CACHE_HOME=/tmp/.cache

USER 10001:10001

EXPOSE 3000

CMD ["sports-api", "--listen", "0.0.0.0:3000", "--database-url", "sqlite:///data/sports-api.db", "--source-fetch-mode", "fixture"]
