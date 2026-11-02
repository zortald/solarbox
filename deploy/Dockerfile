# Image for running solarbox on this Mac or another host (RouterOS images come
# from build-router.sh). Builds a fully static musl binary with the dashboard
# embedded and drops it into a minimal Alpine image. Alpine is musl-native, so
# on Apple Silicon this compiles aarch64-unknown-linux-musl with no cross setup.
#
#   container build -f deploy/Dockerfile -t solarbox:arm64 .
#   container run -d --name solarbox -p 8080:8080 -v solarbox-data:/data solarbox:arm64

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
