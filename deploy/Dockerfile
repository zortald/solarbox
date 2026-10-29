# Build a fully static musl binary with the dashboard embedded, then drop it
# into a minimal Alpine image. Alpine is musl-native, so on an arm64 build host
# (Apple Silicon) this compiles aarch64-unknown-linux-musl with no cross setup.
#
#   docker buildx build --platform linux/arm64 -f deploy/Dockerfile -t solarbox:arm64 --load .

FROM rust:alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY server/ ./
# rust-embed reads server/static at compile time (CARGO_MANIFEST_DIR = /src).
RUN cargo build --release

FROM alpine:3
RUN mkdir -p /data
COPY --from=build /src/target/release/solarbox /usr/local/bin/solarbox
ENV SOLARBOX_DB=/data/solarbox.db \
    SOLARBOX_ADDR=0.0.0.0:8080 \
    SOLARBOX_RETENTION_DAYS=0
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/solarbox"]
