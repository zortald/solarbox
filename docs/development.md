# Development

Standalone solar monitoring for off-grid/hybrid sites: ESP boards push readings
to a small Rust server that runs in a container on the site's own router
(MikroTik/RouterOS), with a dependency-free web dashboard, automations,
Telegram and a setup wizard in progress. UI and all user-facing text are in
**Russian**; code, comments and commit messages in English.

## Rules

- **Nothing about any site goes into the repo**: no SSIDs, site names, IPs of
  real sites, MAC addresses of real devices, tunnel paths, people's names or
  chat ids. Per-site values live in `deploy/local.env`,
  `deploy/site-config.json`, `firmware/site/` (all ignored) and the router's
  data volume.
- **No board is flashed without the site owner's explicit go.** A local
  compile is not a flash.
- **Never fire real notifications or real hardware actions to test.** Telegram
  goes to real people. Test with unit tests, stubbed data in a headless browser,
  or log-only paths.
- **Nothing is written to a BMS or inverter without the owner.** Propose
  values; the owner applies them.
- Solarbox is independent of Home Assistant and does not reuse its config.
- Restarting the server must not re-trigger anything (automation state is
  persisted in `auto_state`). Keep it that way.

## Layout

| Path | What |
|---|---|
| `server/` | Rust (axum, tokio current_thread, rusqlite bundled). `src/*.rs`, UI embedded from `static/` |
| `server/static/` | Dashboard: plain JS modules per page (`flow.js`, `stats.js`, …), own canvas chart `plot.js`, `core.js` = shared helpers (`SB.*`) |
| `firmware/` | ESPHome configs. `gen-jk-ble.py` generates `jk-ble.yaml` (generic JK BMS board, N slots) |
| `deploy/` | `build-router.sh` (armv5/armv7/arm64/x86_64 images), `push-server.sh`, `push-ui.sh`, `push-config.sh`, CI build scripts, RouterOS guide |
| `installer/`, `install/` | Installer CLI (Rust) and its one-line launchers |
| `relay/` | Build relay (Cloudflare Worker) |
| `docs/` | `setup-wizard.md` (design + status), this file; `ROADMAP.md` at the root |

Key server modules: `energy.rs` (per-packet integration → `energy_daily`,
`energy_hourly`, `energy_ext` peaks), `money.rs` (tariffs, day/night),
`outages.rs`, `automations.rs` (rules, journal, persisted state), `auth.rs`
(router-login sessions, emergency password), `tls.rs` (self-signed HTTPS on
the same port as plain ingest), `router.rs` (RouterOS REST), `changes.rs`
(every settings change → Telegram, optional), `telegram.rs`, `discovery.rs`
(boards announce themselves), `ota.rs` (native ESPHome OTA client), `tz.rs`
(Kyiv + EU DST without tz data).

## Build, test, deploy

- On macOS the scripts use Apple `container` (Homebrew; `container system
  start` once after boot) rather than Docker.
- Tests (no local Rust needed):
  `cd server && container run --rm -c 8 -m 4G -v "$PWD":/src -w /src -v sb-cargo:/usr/local/cargo/registry rust:alpine sh -c "apk add -q musl-dev; cargo test --release"`
- Server to the router: `./deploy/push-server.sh` (needs `deploy/local.env`),
  then `SB_ADMIN=… ./deploy/push-ui.sh --reset $(ls server/static)` so the
  embedded UI is served.
- UI only, live without restart: `SB_ADMIN=… ./deploy/push-ui.sh file.js …`
  (`SB_ADMIN` = the emergency password; scripts talk https with `SB_PIN` from
  `deploy/local.env`, printed by the script when missing).
- Firmware compile check (local only):
  `container run --rm -m 4G -v <dir>:/config ghcr.io/esphome/esphome compile jk-ble.yaml`
  (site-built configs also need their `secrets.yaml`).
- Check UI changes in a real browser: headless Chromium via `node:22-alpine` +
  `apk add chromium` + `playwright-core`, screenshots to a temp dir. Stub
  `/api/latest` with `page.route` to show states that aren't happening live.
- `node --check` every JS file you edit (CI does too).

## Releases and on-demand builds

- A release is a `vX.Y.Z` tag. Nothing is built when it is pushed: artifacts
  are built when first asked for and kept in the release.
- `.github/workflows/build.yml` builds one artifact (`kind`, `arch`,
  `version`), signs every file with ed25519 (secret `SB_SIGNING_KEY`, public
  key `server/signing-key.pub`) and uploads it, signature last. Run by hand:
  `gh workflow run build.yml -f kind=server -f arch=armv5 -f version=v0.1.0`.
- `relay/` is the Cloudflare Worker in front of it (`GET /v1/server/<arch>`):
  built → links, not yet → starts the build. Tests: `node --test relay/test.mjs`.
  Deploy: `cd relay && npx wrangler@4 deploy`; its GitHub token (fine-grained,
  this repo only, Actions read/write) is set with `npx wrangler@4 secret put GH_TOKEN`.
- Installer: `installer/` (Rust CLI), built per OS on demand like the server
  (`/v1/installer/<os-arch>`); launchers `install/install.sh` and
  `install/install.ps1`, served by the relay as `/install.sh` and `/install.ps1`.
  Test against RouterOS CHR in QEMU with `--router 127.0.0.1 --rest-port …
  --ssh-port … --lan-ip <CHR's address> --solarbox-url …`; unattended runs:
  `--user <login> --yes` with `SOLARBOX_ROUTER_PASSWORD`.
- Verify a file: `openssl pkeyutl -verify -pubin -inkey <pem> -rawin -in <file> -sigfile <file>.sig`
  (the PEM is `server/signing-key.pub` with the ed25519 SPKI prefix).

## Conventions

- Match the surrounding style; comment density as in the file.
- Estimates are labelled as estimates (≈, «оценка»). The inverter has no grid
  meter: grid power and money are balance estimates.
- Commit after each finished change.
- Firmware is produced by generators that state what is flashed and for what
  (chip, inverter driver, battery slots). Extend generators instead of writing
  one-off YAML. Pins are fixed per board («распиновка Solarbox»); everything
  else (Wi-Fi, server URL, BMS MAC/protocol, names) is runtime.
