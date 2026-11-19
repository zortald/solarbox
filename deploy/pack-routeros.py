#!/usr/bin/env python3
"""Pack one static binary into a RouterOS container image.

  pack-routeros.py <binary> <platform> <arch> <out.tar>     platform: arm/v5, arm/v7, arm64, amd64

A legacy `docker save` tar (uncompressed <dir>/layer.tar referenced from
manifest.json): RouterOS rejects OCI-layout exports with "could not load next
layer", and no image builder is needed for a single static binary.
"""
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
