#!/bin/sh
# Build the 32-bit ARMv7 image (for a RouterOS container on hAP ax S, which
# reports architecture "arm") and export it as a tar.
#   ./deploy/build-armv7.sh
set -e

IMAGE="solarbox:armv7"
OUT="solarbox-armv7.tar"

echo ">> building $IMAGE (linux/arm/v7)"
docker buildx build --platform linux/arm/v7 -f deploy/Dockerfile -t "$IMAGE" --load .

echo ">> exporting image to $OUT"
docker save "$IMAGE" -o "$OUT"

ls -lh "$OUT"
echo ">> done. Upload to the router and import — see deploy/main-router-setup.md"
