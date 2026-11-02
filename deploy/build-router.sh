#!/bin/sh
# Build the server image for a RouterOS container with Apple `container`.
#   ./deploy/build-router.sh [armv5|armv7|arm64]   → solarbox-<arch>.tar
# armv5 = EN7562CT boards (hAP ax S, hEX refresh): RouterOS reports "arm"
# there but only runs arm32v5 images.
#
# The static musl binary is cross-compiled in a musl toolchain container
# running natively (no emulation), then packed by hand into an image with
# that one file: a legacy `docker save` tar (uncompressed <dir>/layer.tar
# referenced from manifest.json). RouterOS rejects OCI-layout exports with
# "could not load next layer", and no image builder is needed for a single
# static binary.
set -e
cd "$(dirname "$0")/.."

ARCH=${1:-armv5}
case $ARCH in
  armv5) TOOL=armv5te-musleabi;  TARGET=armv5te-unknown-linux-musleabi;  PLATFORM=arm/v5 ;;
  armv7) TOOL=armv7-musleabihf;  TARGET=armv7-unknown-linux-musleabihf;  PLATFORM=arm/v7 ;;
  arm64) TOOL=aarch64-musl;      TARGET=aarch64-unknown-linux-musl;      PLATFORM=arm64 ;;
  *) echo "usage: $0 [armv5|armv7|arm64]" >&2; exit 2 ;;
esac
OUT="$(pwd)/solarbox-$ARCH.tar"

container system status >/dev/null 2>&1 || { echo ">> run: container system start" >&2; exit 1; }

echo ">> compiling $TARGET"
# The toolchain image defaults CARGO_BUILD_TARGET to $TARGET. rust-embed reads
# server/static at compile time. sb-cargo caches the crate registry.
container run --rm -c "$(sysctl -n hw.ncpu)" -m 4G \
  -v "$(pwd)/server":/src -w /src -v sb-cargo:/root/.cargo/registry \
  "messense/rust-musl-cross:$TOOL" cargo build --release

python3 - "server/target/$TARGET/release/solarbox" "$PLATFORM" "$ARCH" "$OUT" <<'PY'
import hashlib, io, json, sys, tarfile, time
binary, platform, arch, out = sys.argv[1:]
now = int(time.time())

def entry(name, mode, data=None):
    t = tarfile.TarInfo(name)
    t.mode, t.mtime = mode, now
    if data is None:
        t.type = tarfile.DIRTYPE
        return t, None
    t.size = len(data)
    return t, io.BytesIO(data)

# Root filesystem: /solarbox and the WORKDIR /data (the data mount goes there).
layer = io.BytesIO()
with tarfile.open(fileobj=layer, mode="w", format=tarfile.USTAR_FORMAT) as t:
    t.addfile(*entry("data/", 0o755))
    t.addfile(*entry("solarbox", 0o755, open(binary, "rb").read()))
layer = layer.getvalue()

os_arch, _, variant = platform.partition("/")
config = {
    "architecture": os_arch, "os": "linux",
    "config": {
        "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "SOLARBOX_DB=/data/solarbox.db",
                "SOLARBOX_ADDR=0.0.0.0:8080",
                "SOLARBOX_RETENTION_DAYS=0"],
        "Entrypoint": ["/solarbox"],
        "WorkingDir": "/data",
        "ExposedPorts": {"8080/tcp": {}},
    },
    "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now)),
    "rootfs": {"type": "layers", "diff_ids": ["sha256:" + hashlib.sha256(layer).hexdigest()]},
}
if variant:
    config["variant"] = variant
manifest = [{"Config": "config.json", "RepoTags": [f"solarbox:{arch}"], "Layers": ["l0/layer.tar"]}]

with tarfile.open(out, "w", format=tarfile.USTAR_FORMAT) as t:
    t.addfile(*entry("manifest.json", 0o644, json.dumps(manifest).encode()))
    t.addfile(*entry("config.json", 0o644, json.dumps(config, indent=1).encode()))
    t.addfile(*entry("l0/", 0o755))
    t.addfile(*entry("l0/layer.tar", 0o644, layer))
PY
ls -lh "$OUT"
