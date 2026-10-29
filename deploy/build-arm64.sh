#!/bin/sh
# Build the arm64 image for a RouterOS container and export it as a tar.
# Run from the repo root:  ./deploy/build-arm64.sh
set -e

IMAGE="solarbox:arm64"
OUT="solarbox-arm64.tar"

echo ">> building $IMAGE (linux/arm64)"
docker buildx build --platform linux/arm64 -f deploy/Dockerfile -t "$IMAGE" --load .

echo ">> exporting image to $OUT"
docker save "$IMAGE" -o "$OUT"

ls -lh "$OUT"
echo ">> done. Import on the router — see deploy/routeros.md"
