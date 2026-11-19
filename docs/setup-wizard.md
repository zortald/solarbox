# Setup wizard and drivers — design

Goal: a fresh Solarbox greets the owner («Вас приветствует Solarbox») and walks
through everything — site, boards (what to buy, how to solder, how to flash),
battery, inverter, notifications — with no ESPHome toolchain, no laptop
config files and no site-specific firmware builds.

## Principles

- **No site data in firmware; built on demand.** Only what is chosen — chip,
  inverter driver, BMS type and count — is compiled in. Wi-Fi, the Solarbox
  address, BMS Bluetooth addresses, names are set at runtime and kept in the
  board's flash. An image is built the first time someone asks for that
  combination and served to everyone after that (see *Builds and delivery*);
  nobody compiles anything per site.
- **Boards announce themselves.** Every ingest packet carries a small `board`
  block (type, firmware version, MAC, IP). Unknown boards show up in the wizard
  as «новая плата» and are adopted with one click (added to `config.json`).
- **Zero-config address.** With the Solarbox URL left empty a board pushes to
  `http://<its default gateway>:8080/api/ingest` — correct whenever Solarbox
  runs on the site router (the normal case). Otherwise the wizard (or the
  board's own web page) sets the URL.
- **Drivers, not special cases.** Everything type-specific lives in a driver;
  the rest of Solarbox only sees canonical metrics.

## Drivers

A driver = firmware image(s) + wiring guide + metric mapping + settings UI.

| Kind | Driver | Board | Link | Status |
|---|---|---|---|---|
| inverter | `pi30` — Voltronic/PowMr/MPP/EASun, PI30 protocol | ESP8266 D1 mini + RS232 (MAX3232) | RJ45 RS232, 2400 Bd | first |
| bms | `jk-ble` — JK BMS (JK02_24S/JK02_32S/JK04) | ESP32-C3 | Bluetooth LE | first |
| inverter | `deye-modbus` — Deye/Sunsynk hybrids | ESP32 + RS485 | Modbus RTU 9600 Bd | later |
| bms | `jk-rs485`, `pace`, `seplos`… | — | RS485/CAN | later |

Canonical metrics every inverter driver should provide where it can:
`pv_w, pv_v, ac_out_w, ac_out_v, grid_v, grid_hz, batt_v, batt_a, temp`.
Battery drivers: `soc, bms_v, bms_a, bms_w, cap_ah, c1…cN, r1…rN, t1, t2,
cell_min, cell_max, cell_delta, chg, dsg, bal`.
Energy, outages, automations and the main page work from these only.

Driver descriptors live in `server/static/drivers/*.js` (UI: wizard steps,
wiring SVG, settings panel) and their images in `/data/fw/<driver>-<chip>.bin`
(plus the embedded copy shipped with the server).

## Board firmware contract

- Wi-Fi: no networks compiled in. First boot → own access point
  `solarbox-<type>-XXXX` with captive portal; right after a USB flash
  web.esphome.io offers Wi-Fi setup over Improv-Serial.
- Runtime settings (template `text`/`number` entities, restored from flash):
  `Solarbox URL` (empty = gateway:8080), driver-specific ones (`BMS MAC`, …).
- Push every 5–10 s: `{ device, board: {type, fw, mac, ip}, metrics }`.
- OTA: ESPHome native, no password (Solarbox's OTA client), web_server.

## Wizard steps

1. Welcome → equipment (inverter, BMS type and count, system voltage).
2. Boards — recommended, or planned from what the owner already has.
3. Site — name, time zone.
4. Assembly — per board: shopping list, wiring diagram, «Скачать прошивку»
   (built on demand) + web.esphome.io walkthrough (USB flash, Wi-Fi over
   Improv). Waits until the board announces itself; boards are matched by MAC.
5. Battery — BLE scan list from the board, pick the packs, capacity,
   SOC 100 % / 0 % thresholds (checked against the inverter's charge voltage).
6. Inverter — battery type and voltages proposed from the battery.
7. Tariff, Telegram (optional).
8. **Review → «Применить».** Nothing is applied while stepping through: answers
   are a draft kept on the server, so an interrupted wizard resumes where it
   stopped. The last screen lists everything that will change, grouped
   (Solarbox settings, boards to adopt, settings written to each board, BMS,
   inverter) with old → new values. One «Применить», then per-item progress;
   a failed item is shown and can be retried, the rest stays applied.
9. Done → the emergency password is shown once, to write down.

Login is the router's own (as for the dashboard); there is no separate admin
password step. The wizard opens by itself while there are no boards and it was
not finished or skipped; later it is in Settings. `?demo` keeps the simulated
mode (fake boards and BMS, nothing saved or sent) for headless checks.

## Builds and delivery

One rule for board firmware, server images and the installer: **built on
demand, reused once built**.

- Solarbox (or the installer) asks a Cloudflare Worker for a combination from
  a fixed list (never arbitrary YAML). Built already → link to the release
  asset. Not yet → the Worker starts a GitHub Actions build
  (`esphome/build-action` for boards, the musl cross build for the server) and
  the caller polls until it is there. The GitHub token lives only in the Worker.
- Artifacts go to the releases of this repo (public), downloadable without a
  token. Every artifact is signed in CI with our key (kept in GitHub secrets);
  Solarbox installs only signed files — itself and board images before OTA.
- The ESPHome version is pinned in the repo; a new one only after the bench.
  Builds of the last three versions are kept; older ones are rebuilt on demand.
- Fallback for people who want their own changes: download the YAML and build
  it locally.

## Install

- One line in a terminal: `irm …/install.ps1 | iex` (Windows) or
  `sh -c "$(curl -fsSL …/install.sh)"` (macOS, Linux, Android/Termux). These
  only pick and fetch the installer CLI (Rust, one code base, built per OS on
  demand like everything else).
- The CLI finds the router, asks its password (used for the install only, never
  stored), checks RouterOS version, architecture, memory and storage, and asks
  before installing. It has the router fetch the `container` package from
  MikroTik itself, uploads the server image over SSH, waits for the physical
  button press of `device-mode` (30 min, continues by itself), sets up
  networking and the firewall rule, creates a read-only service user with a
  random password, and prints the address to open.
- Unsuitable router → says why and offers Docker on any Linux box (then boards
  get the Solarbox address in the wizard, and the emergency password is shown).
- Later: iPhone (a line for the MikroTik app terminal), OpenWrt (a router
  driver behind the same seam as RouterOS: login, remote access, system info).

## Updates

- Solarbox checks for new versions once a day and only offers them.
- «Обновить» → Solarbox downloads its new signed binary into `/data`, replaces
  itself in place (no container or router change, no extra router rights) and
  rolls back to the previous one if the new one is not healthy within 5 min.
  Restarting re-triggers nothing (automation state is persisted). The container
  image is only a starter; changing it means running the installer again.
- Boards: the same, per board, by button.

## Bench

Development never touches site 1. The bench: an ESP32-S3 on USB (its previous
firmware backed up off-repo first), the server on the Mac, a JK BMS emulated by
the Mac over Bluetooth with frames from esphome-jk-bms, the inverter later
through a USB-UART bridge made from another ESP, RouterOS CHR in a VM for the
installer. Site 2 is the first real install.

## Migrating boards that run site-specific firmware

ESPHome keys the saved Wi-Fi by the config hash when networks are compiled in,
and by a constant when none are. The generic images have none, so Wi-Fi set
once survives every later update — but a board moving *from* a site build
(compiled networks) to a generic image would come up with no Wi-Fi. Remote
boards therefore get a transitional site build first that also writes the
current network under the constant key, then the generic image.

## Status

- [x] Design (this file)
- [x] `firmware/jk-ble.yaml` — generic JK BMS board, generated by `gen-jk-ble.py` with N battery slots (runtime MAC/protocol/name per slot; 2 slots compile on ESP32-C3, 80 % flash; not yet on hardware)
- [x] Server: `board` block in ingest, `GET /api/discovered`
- [x] Wizard UI prototype (`wizard.html`, simulated, `?demo` only)
- [x] Repo history check → AGPL-3.0, public
- [ ] Server: adopt by MAC, discovery per slot, site name, draft + review/apply, wizard autostart, writing board settings, router seam
- [ ] `gen-board.py` for every combination (incl. S3, generic PI30)
- [x] On-demand builds for the server: `build.yml`, signing, releases, relay Worker (board images next)
- [ ] Bench: S3 end-to-end, emulated JK
- [ ] Wizard wired to the real server, step by step
- [ ] Self-update with rollback
- [ ] Installer CLI + launch lines, tested on CHR
