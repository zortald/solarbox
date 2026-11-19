#!/bin/sh
# Build the server image for a RouterOS container with Apple `container`.
#   ./deploy/build-router.sh [armv5|armv7|arm64|x86_64]   → solarbox-<arch>.tar
# armv5 = EN7562CT boards (hAP ax S, hEX refresh): RouterOS reports "arm"
# there but only runs arm32v5 images.
#
# The static musl binary is cross-compiled in a musl toolchain container
# running natively (no emulation), then packed by hand into an image with
# that one file by deploy/pack-routeros.py.
set -e
cd "$(dirname "$0")/.."

ARCH=${1:-armv5}
case $ARCH in
  armv5) TOOL=armv5te-musleabi;  TARGET=armv5te-unknown-linux-musleabi;  PLATFORM=arm/v5 ;;
  armv7) TOOL=armv7-musleabihf;  TARGET=armv7-unknown-linux-musleabihf;  PLATFORM=arm/v7 ;;
  arm64) TOOL=aarch64-musl;      TARGET=aarch64-unknown-linux-musl;      PLATFORM=arm64 ;;
  x86_64) TOOL=x86_64-musl;      TARGET=x86_64-unknown-linux-musl;       PLATFORM=amd64 ;;
  *) echo "usage: $0 [armv5|armv7|arm64|x86_64]" >&2; exit 2 ;;
esac
OUT="$(pwd)/solarbox-$ARCH.tar"

container system status >/dev/null 2>&1 || { echo ">> run: container system start" >&2; exit 1; }

echo ">> compiling $TARGET"
# The toolchain image defaults CARGO_BUILD_TARGET to $TARGET. rust-embed reads
# server/static at compile time. sb-cargo caches the crate registry.
container run --rm -c "$(sysctl -n hw.ncpu)" -m 4G \
  -v "$(pwd)/server":/src -w /src -v sb-cargo:/root/.cargo/registry \
  "messense/rust-musl-cross:$TOOL" cargo build --release

python3 deploy/pack-routeros.py "server/target/$TARGET/release/solarbox" "$PLATFORM" "$ARCH" "$OUT"
ls -lh "$OUT"
