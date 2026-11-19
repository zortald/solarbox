# solarbox

A tiny, self-contained telemetry collector + dashboard for an off-grid
inverter/battery site. **Push-based**: the ESP boards POST their readings; the
server never polls anything. One static binary, SQLite storage, dashboard baked
in — meant to live in a container on the site's own router, with no dependency
on any home automation system.

```
boards ──POST JSON──▶ /api/ingest ──▶ SQLite ──▶ dashboard (/,  embedded)
                                           ▲
                                   /api/latest · /api/series · /api/catalog
```

## Install

On a computer in the same network as the MikroTik router (RouterOS 7.25+, a
model with container support):

```
sh -c "$(curl -fsSL https://get.solarbox.workers.dev/install.sh)"   # macOS, Linux, Android (Termux)
irm https://get.solarbox.workers.dev/install.ps1 | iex              # Windows (PowerShell)
```

The installer asks for the router's password (used for this run only), shows
what it will add and asks before doing it. One step needs you at the router:
RouterOS only enables containers after its power is cut once.

## Components

| Dir          | What                                                             |
|--------------|-----------------------------------------------------------------|
| `server/`    | Rust (axum + rusqlite, SQLite bundled), dashboard embedded.     |
| `server/static/` | Dependency-free dashboard (own canvas chart, no CDN).       |
| `firmware/`  | ESPHome board firmware (generated) + how boards push to `/api/ingest`. |
| `deploy/`    | Router image build (armv5/armv7/arm64), push scripts, RouterOS guide. |

## HTTP API

- `POST /api/ingest` — `{ "device": "...", "ts"?: unixsec, "metrics": { k: n } }`
  (or an array). Optional `X-Token` header if the server has `SOLARBOX_TOKEN`.
- `GET  /api/latest` — newest value per device/metric (the "now" view).
- `GET  /api/series?device=&metric=&from=&to=&points=` — time series, averaged
  into buckets when the range is wide.
- `GET  /api/catalog` — devices/metrics seen.
- `GET  /healthz`, `GET /` + static dashboard.

## Config (env)

| Var | Default | |
|-----|---------|-|
| `SOLARBOX_DB` | `solarbox.db` | SQLite path (container: `/data/solarbox.db`) |
| `SOLARBOX_ADDR` | `0.0.0.0:8080` | listen address |
| `SOLARBOX_TOKEN` | *(unset)* | if set, ingest requires `X-Token` |
| `SOLARBOX_TIERS` | *(see below)* | age→resolution downsampling tiers |
| `SOLARBOX_COMPACT_EVERY` | `1h` | how often the compaction pass runs |
| `SOLARBOX_COMPACT_TAIL` | `2d` | lookback for the open-ended oldest tier |
| `SOLARBOX_RETENTION_DAYS` | `0` | optional hard delete beyond compaction (`0` = keep) |

## Storage: tiered downsampling

Instead of deleting history, solarbox thins it by age — recent data stays
fine-grained, old data is averaged into ever-coarser buckets (capped). Readings
never vanish; their resolution just drops as they age, so the DB stays small
while keeping long history. `latest` (the "now" view) is never touched.

Default `SOLARBOX_TIERS` (age → bucket):

```
today 20s · 1d 1m · 2d 5m · 3d 10m · 4d 15m · 5d 20m · 6d 25m · 7d+ 30m
# i.e. 0:20s,1d:1m,2d:5m,3d:10m,4d:15m,5d:20m,6d:25m,7d:30m
```

A compaction pass runs every `SOLARBOX_COMPACT_EVERY`; `POST /api/compact`
forces one. The pass is idempotent — a band already on its grid is skipped.

## Build / run

Local (needs Rust): `cd server && cargo run`.

For the router (no local Rust needed — builds in Apple `container`):

```
./deploy/build-router.sh armv5   # → solarbox-armv5.tar (or armv7 / arm64)
```

then import per [`deploy/routeros.md`](deploy/routeros.md).

## Portability

It's one binary + a SQLite file. Runs the same on a Mac, a mini-PC, or a
RouterOS container — point the boards at wherever it listens.

## License

AGPL-3.0-or-later — see `LICENSE`. Changed versions you distribute or let
others use over a network must be published under the same license.
