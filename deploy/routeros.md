# Running solarbox in a RouterOS container

## Pick the image

| Router CPU (`/system/resource/print` → architecture) | Build |
|---|---|
| `arm` (e.g. hAP ax S, EN7562CT) | `./deploy/build-router.sh armv5` → `solarbox-armv5.tar` |
| `arm` with ARMv7 cores | `./deploy/build-router.sh armv7` |
| `arm64` | `./deploy/build-router.sh arm64` |

The build runs in Apple `container` (`brew install container`,
`container system start`) and writes the legacy `docker save` format:
RouterOS rejects OCI-layout exports with "could not load next layer".

## 1. Container package

Let the router fetch the package itself (uploading the npk by hand is unreliable):

```
/system/package/update/check-for-updates
/system/package/enable container
/system/package/apply-changes          ;# downloads + reboots
```

Do this **before** device-mode: any reboot cancels a pending device-mode update.

## 2. device-mode container=yes (needs physical access)

```
/system/device-mode/update container=yes activation-timeout=30m
```

- The pending update exists only while the session that ran it stays open.
  REST times out at 60 s and a one-shot `ssh host "cmd"` closes, so run it in an
  interactive ssh (inside tmux) and keep it open until the router reboots.
- Confirm by cutting **all** power for ~10 s (DC jack and PoE if fed), or a
  brief tap of Reset while running (a long hold at boot resets the config).
- **attempt-count** (max 3): every update with parameters counts, including
  refused ones. At 3 a power cut alone does not reset it — send the update
  again, and cut power while the "too many unsuccessful attempts" window is
  open → 0. Then arm again and cut power a second time.

## 3. Networking

```
/interface/veth/add name=veth-sb address=172.18.0.2/24 gateway=172.18.0.1
/interface/bridge/add name=cont-br
/ip/address/add address=172.18.0.1/24 interface=cont-br
/interface/bridge/port/add bridge=cont-br interface=veth-sb
/ip/firewall/nat/add chain=dstnat action=dst-nat protocol=tcp dst-address=<router-lan-ip> \
    dst-port=8080 to-addresses=172.18.0.2 to-ports=8080 comment="solarbox"
```

The boards must be able to reach `<router-lan-ip>:8080` to push.

Solarbox checks logins and opens remote access through the router's REST API.
The default input firewall drops everything not from the LAN list, and
`cont-br` is not in it, so let the container reach REST (port 80, service
`www`) and nothing else:

```
/ip/firewall/filter/add chain=input action=accept protocol=tcp src-address=172.18.0.2 \
    dst-address=172.18.0.1 dst-port=80 comment="solarbox: REST роутера" place-before=0
```

Don't add `cont-br` to the LAN list instead: that would open every router
service (SSH, Winbox…) to a container that may face the internet.

## 4. Container

RouterOS 7.25 syntax (`list=` / `mountlists=`):

```
/container/mounts/add list=sb-data src=/sb-data dst=/data
/container/add file=solarbox-armv5.tar interface=veth-sb root-dir=sb-root \
    mountlists=sb-data logging=yes start-on-boot=yes name=solarbox
/container/start solarbox
```

Optional env (`/container/envs/add list=sb-env key=… value=…`, then
`envlists=sb-env`): `SOLARBOX_ADMIN_PASSWORD` sets the emergency password
(otherwise one is generated into `/data/admin-password` and printed once to the
container log), `SOLARBOX_TOKEN` makes ingest require `X-Token`.

Dashboard `https://<router-lan-ip>:8080/` (self-signed certificate, accept it
once per device after comparing the fingerprint on the «Доступ» page; plain
`http://` redirects). Log in with the router's own login and password, or with
an empty login and the emergency password. Boards keep pushing over plain HTTP
to the same port. Health `/healthz` (both). Details: `docs/remote-access.md`.


## Updating

`./deploy/push-server.sh` builds, uploads, stops, repulls and starts the
container, then removes the tar from flash. Settings for your router go in
`deploy/local.env` (not committed, see `deploy/local.env.example`). The
database, `config.json`, secrets and UI overrides live in the `sb-data` mount
and survive.

UI-only changes: `./deploy/push-ui.sh [files]` — live, no rebuild.

## Notes

- Don't format USB storage that holds the owner's files; internal flash is
  enough (the DB is capped at 40 MB by default).
- The binary is ~3 MB and uses ~4 MB RAM.
