# Running solarbox in a RouterOS container (hAP ax S, arm64)

> Containers on RouterOS need **device-mode `container=yes`**, which requires a
> physical power-cycle (hold reset / cold boot) to confirm. Plan that in — it
> cannot be enabled purely over the network.

## Where to run it

Two options, depending on where the ESP boards can reach:

- **On `.230`** — `.230` is a Wi-Fi *client* on the boards' LAN
  (`192.168.88.0/24`), i.e. on the same subnet as the boards. The boards can
  POST straight to `.230`'s LAN address with no routing. **Recommended** so the
  push path is trivial.
- **On the main router `192.168.88.1`** — only if the boards have a working route
  to it (they currently sit behind the boards' router's NAT, so this needs a reverse route
  first).

Reach the dashboard from your Mac through the existing tunnel by
adding a port-forward to `<router>:8080`.

## One-time: enable containers

```
/system/device-mode/update container=yes
# → confirm with a power-cycle within the time window RouterOS prints
```

Add a veth + bridge + address for the container, and a NAT/forward so the
container port is reachable:

```
/interface/veth/add name=veth-sb address=172.17.0.2/24 gateway=172.17.0.1
/interface/bridge/add name=dockers
/ip/address/add address=172.17.0.1/24 interface=dockers
/interface/bridge/port add bridge=dockers interface=veth-sb
# publish 8080 to the LAN
/ip/firewall/nat/add chain=dstnat dst-port=8080 protocol=tcp action=dst-nat to-address=172.17.0.2 to-port=8080
```

Mount for the SQLite file (survives container restarts/upgrades):

```
/container/mounts/add name=sb-data src=/sb-data dst=/data
```

## Import the image

Build + export on the Mac first:

```
./deploy/build-arm64.sh        # produces solarbox-arm64.tar
```

Upload `solarbox-arm64.tar` to the router (Files), then:

```
/container/add file=solarbox-arm64.tar interface=veth-sb root-dir=sb-root \
    mounts=sb-data envlist=sb-env logging=yes
/container/envs/add name=sb-env key=SOLARBOX_RETENTION_DAYS value=90
# optional shared secret for ingest:
# /container/envs/add name=sb-env key=SOLARBOX_TOKEN value=<secret>
/container/start [find]
```

Check it:

```
/container/print
/log/print where topics~"container"
```

Dashboard: `http://<router-lan-ip>:8080/`  ·  health: `/healthz`

## Notes

- `SOLARBOX_DB` defaults to `/data/solarbox.db` (the mounted volume).
- If flash is tight, lower `SOLARBOX_RETENTION_DAYS`.
- The image is ~10–15 MB; the binary itself is a few MB.
