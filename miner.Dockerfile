# syntax=docker/dockerfile:1
FROM rust:1.80.1 AS builder

WORKDIR /usr/src/btc_rust
COPY . .

RUN rustup target add x86_64-unknown-linux-musl \
    && cargo build --release --bin miner --target-dir /btc_miner/target --target x86_64-unknown-linux-musl

FROM scratch

COPY --from=builder /btc_miner/target/x86_64-unknown-linux-musl/release/miner /btc_miner

VOLUME [ "/data" ]

ENTRYPOINT [ "/btc_miner" ]
