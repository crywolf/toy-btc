# syntax=docker/dockerfile:1
FROM rust:1.86-slim AS builder

RUN apt-get -y update && apt-get install -y protobuf-compiler

WORKDIR /usr/src/btc_rust

COPY ./lib ./lib
COPY ./miner ./miner
COPY ./node ./node
COPY ./wallet ./wallet
COPY ./proto ./proto
COPY ./build.rs .
COPY ./Cargo.toml .
COPY ./Cargo.lock .

RUN rustup target add x86_64-unknown-linux-musl \
    && cargo build --locked --release --bin node --target-dir /btc_node/target --target x86_64-unknown-linux-musl

FROM scratch

COPY --from=builder /btc_node/target/x86_64-unknown-linux-musl/release/node /btc_node

VOLUME [ "/data" ]

ENTRYPOINT [ "/btc_node" ]
