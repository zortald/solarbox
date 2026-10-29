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

## Components

| Dir          | What                                                             |
|--------------|-----------------------------------------------------------------|
| `server/`    | Rust (axum + rusqlite, SQLite bundled), dashboard embedded.     |
| `server/static/` | Dependency-free dashboard (own canvas chart, no CDN).       |
| `firmware/`  | The two ESPHome board configs + how they push to `/api/ingest`. |
| `deploy/`    | Dockerfile, arm64 build script, RouterOS container guide.       |

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
| `SOLARBOX_COMPACT_EVERY` | `6h` | how often the compaction pass runs |
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

For the router (no local Rust needed — builds in Docker, arm64-native on Apple
Silicon):

```
./deploy/build-arm64.sh      # → solarbox-arm64.tar
```

then import per [`deploy/routeros.md`](deploy/routeros.md).

## Portability

It's one binary + a SQLite file. Runs the same on a Mac, a mini-PC, or a
RouterOS container — point the boards at wherever it listens.
