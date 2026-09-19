# ApeMe indexer. Build: docker compose build indexer (from apme-ops). Runs `ape-indexer stream` on the host network.
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# dependency layer: cache the crate build separately from our source
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs && cargo build --release && rm -rf src target/release/deps/ape_indexer*
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/ape-indexer /usr/local/bin/ape-indexer
COPY healthcheck.sh /usr/local/bin/healthcheck
ENV RUST_LOG=info,sqlx=warn METRICS_ADDR=127.0.0.1:9464
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=6 CMD ["healthcheck"]
ENTRYPOINT ["ape-indexer"]
CMD ["stream"]
