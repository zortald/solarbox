# Setup wizard and drivers — design

Goal: a fresh Solarbox greets the owner («Вас приветствует Solarbox») and walks
through everything — site, boards (what to buy, how to solder, how to flash),
battery, inverter, notifications — with no ESPHome toolchain, no laptop
config files and no site-specific firmware builds.

## Principles

- **One firmware per board type, no site data inside.** Wi-Fi, the Solarbox
  address, the BMS Bluetooth address etc. are set at runtime and kept in the
  board's flash. Solarbox ships the prebuilt images and updates boards over the
  air; nobody compiles anything per site.
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

1. Welcome → site name, time zone, admin password.
2. Boards — per driver: shopping list, wiring diagram, «Скачать прошивку» +
   web.esphome.io walkthrough, Wi-Fi, then waits until the board appears and
   adopts it.
3. Battery — BLE scan list from the board, pick the pack, capacity,
   SOC 100 % / 0 % thresholds (checked against the inverter's charge voltage).
4. Inverter — battery type and voltages proposed from the battery.
5. Telegram (optional) → done.

The wizard opens automatically while `config.json` has no boards; later it is
in Settings and never overwrites a working setup without asking.

## Migrating boards that run site-specific firmware

ESPHome keys the saved Wi-Fi by the config hash when networks are compiled in,
and by a constant when none are. The generic images have none, so Wi-Fi set
once survives every later update — but a board moving *from* a site build
(compiled networks) to a generic image would come up with no Wi-Fi. Remote
boards therefore get a transitional site build first that also writes the
current network under the constant key, then the generic image.

## Status

- [x] Design (this file)
- [x] `firmware/jk-ble.yaml` — generic JK BMS board (compiles; not yet on hardware)
- [x] Server: `board` block in ingest, `GET /api/discovered`
- [ ] Generic PI30 inverter image
- [ ] Prebuilt images served by Solarbox + OTA from the library
- [ ] Wizard UI, wiring diagrams
- [ ] Driver descriptors; inverter settings page behind the `pi30` driver
