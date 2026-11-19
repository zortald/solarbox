#!/bin/sh
# Build the installer CLI for one target on the matching CI runner.
#   ./deploy/ci-build-installer.sh <target> [outdir]
#   targets: linux-x86_64 linux-arm64 (Docker, static musl) · macos-arm64
#   macos-x86_64 (macOS runner) · windows-x86_64 windows-arm64 (Windows
#   runner, bash) · android-arm64 (Linux runner with the Android NDK)
# Writes <outdir>/solarbox-installer-<target>[.exe].
set -e
cd "$(dirname "$0")/.."
T=$1; OUT=${2:-dist}
EXE=
case $T in
  linux-x86_64)   TOOL=x86_64-musl;  TARGET=x86_64-unknown-linux-musl ;;
  linux-arm64)    TOOL=aarch64-musl; TARGET=aarch64-unknown-linux-musl ;;
  macos-arm64)    TARGET=aarch64-apple-darwin ;;
  macos-x86_64)   TARGET=x86_64-apple-darwin ;;
  windows-x86_64) TARGET=x86_64-pc-windows-msvc; EXE=.exe ;;
  windows-arm64)  TARGET=aarch64-pc-windows-msvc; EXE=.exe ;;
  android-arm64)  TARGET=aarch64-linux-android ;;
  *) echo "usage: $0 <target> [outdir]" >&2; exit 2 ;;
esac

if [ -n "$TOOL" ]; then
  # The toolchain image sets CARGO_BUILD_TARGET; the crate reads ../deploy.
  docker run --rm -v "$PWD":/src -w /src/installer "messense/rust-musl-cross:$TOOL" cargo build --release
elif [ "$T" = android-arm64 ]; then
  rustup target add "$TARGET"
  cargo install -q cargo-ndk
  (cd installer && cargo ndk -t arm64-v8a build --release)
else
  rustup target add "$TARGET"
  (cd installer && cargo build --release --target "$TARGET")
fi

mkdir -p "$OUT"
cp "installer/target/$TARGET/release/solarbox-install$EXE" "$OUT/solarbox-installer-$T$EXE"
ls -l "$OUT"
