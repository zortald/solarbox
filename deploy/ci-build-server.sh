#!/bin/sh
# Build the server for one architecture on a Linux host with Docker (CI).
#   ./deploy/ci-build-server.sh <arch> [outdir]      arch: armv5 armv7 arm64 x86_64 mips mipsel
# Writes <outdir>/solarbox-server-<arch> (static binary: OpenWrt, Docker,
# self-update) and, for the architectures RouterOS runs containers on,
# <outdir>/solarbox-server-<arch>.routeros.tar.
# mips is tier 3 in Rust (no prebuilt std): nightly + build-std with zig as the
# cross C compiler and linker; it expects nightly with rust-src and
# cargo-zigbuild on the host.
set -e
cd "$(dirname "$0")/.."
ARCH=$1; OUT=${2:-dist}
case $ARCH in
  armv5)  TOOL=armv5te-musleabi; TARGET=armv5te-unknown-linux-musleabi; PLATFORM=arm/v5 ;;
  armv7)  TOOL=armv7-musleabihf; TARGET=armv7-unknown-linux-musleabihf; PLATFORM=arm/v7 ;;
  arm64)  TOOL=aarch64-musl;     TARGET=aarch64-unknown-linux-musl;     PLATFORM=arm64 ;;
  x86_64) TOOL=x86_64-musl;      TARGET=x86_64-unknown-linux-musl;      PLATFORM=amd64 ;;
  mips)   TARGET=mips-unknown-linux-musl ;;
  mipsel) TARGET=mipsel-unknown-linux-musl ;;
  *) echo "usage: $0 armv5|armv7|arm64|x86_64|mips|mipsel [outdir]" >&2; exit 2 ;;
esac

if [ -n "$TOOL" ]; then
  docker run --rm -v "$PWD/server":/src -w /src "messense/rust-musl-cross:$TOOL" cargo build --release
else
  (cd server && cargo +nightly zigbuild --release -Zbuild-std=std,panic_abort --target "$TARGET")
fi

mkdir -p "$OUT"
BIN="$OUT/solarbox-server-$ARCH"
cp "server/target/$TARGET/release/solarbox" "$BIN"
[ -n "$PLATFORM" ] && python3 deploy/pack-routeros.py "$BIN" "$PLATFORM" "$ARCH" "$BIN.routeros.tar"
ls -l "$OUT"/solarbox-server-"$ARCH"*
