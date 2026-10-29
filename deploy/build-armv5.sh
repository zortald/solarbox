#!/bin/sh
# Build the arm32v5 image for RouterOS containers on EN7562CT boards (hAP ax S)
# and write it as a legacy `docker save` tar — RouterOS fails on Docker
# Desktop's OCI-layout export ("could not load next layer"): it needs
# uncompressed <dir>/layer.tar entries referenced from manifest.json.
#   ./deploy/build-armv5.sh   → solarbox-armv5.tar
set -e

OUT="$(pwd)/solarbox-armv5.tar"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

docker buildx build --platform linux/arm/v5 --provenance=false --sbom=false \
  -f deploy/Dockerfile.armv5 -o type=docker,dest="$WORK/oci.tar" .

mkdir "$WORK/oci" "$WORK/out"
tar xf "$WORK/oci.tar" -C "$WORK/oci"

python3 - "$WORK" <<'PY'
import gzip, json, shutil, sys
work = sys.argv[1]
m = json.load(open(f"{work}/oci/manifest.json"))[0]
shutil.copy(f"{work}/oci/{m['Config']}", f"{work}/out/config.json")
layers = []
for i, path in enumerate(m["Layers"]):
    d = f"{work}/out/l{i}"
    __import__("os").makedirs(d)
    src = open(f"{work}/oci/{path}", "rb")
    head = src.read(2); src.seek(0)
    with open(f"{d}/layer.tar", "wb") as dst:
        shutil.copyfileobj(gzip.open(src) if head == b"\x1f\x8b" else src, dst)
    layers.append(f"l{i}/layer.tar")
json.dump([{"Config": "config.json", "RepoTags": ["solarbox:armv5"], "Layers": layers}],
          open(f"{work}/out/manifest.json", "w"))
PY

cd "$WORK/out"
COPYFILE_DISABLE=1 tar --no-xattrs --format ustar -cf "$OUT" manifest.json config.json l*
ls -lh "$OUT"
