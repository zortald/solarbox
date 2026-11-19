# Solarbox roadmap

Status as of 2026-10-31. Order is the intended order; ask Sergey before
reordering.

## Done

- Push-based server in a RouterOS container (armv5), SQLite with tiered
  thinning, live UI over SSE, hot-reloaded config and UI overrides
- Main page «Потоки»: power flow, house coloured by its sources, grid X on
  outages, time to 1 % / 100 % from the BMS capacity
- Statistics: day / month / 7 d / 30 d / 12 months, exact daily peaks, outages,
  money by tariff with day/night split
- Outages: detection, page, background on charts, persisted alert state
- Automations: conditions on any metric, clock, energy; Telegram actions;
  journal; state survives restarts
- Telegram: several recipients, access requests, command menu, every settings
  change mirrored (switchable)
- Inverter (PI30) settings page; BMS SOC calibration panel
- Boards: native OTA from the dashboard; boards announce themselves
- Generic JK BMS board firmware with N slots (generator, compiles; not on hardware)
- Login with the router's own credentials, self-signed HTTPS on the same port,
  page «Доступ»: one-click port forward to the internet (docs/remote-access.md)

## Next

1. **Battery bank** — several batteries on one inverter: config lists them,
   server sums power/Ah, SOC by Ah, main page / energy / money / outages /
   Telegram use the bank; battery page with a bank summary and a tab per pack.
   First user: a second site with one PI30 inverter and two JK BMS side by side.
2. **Firmware generator with modules** — board (ESP32-S3 / C3 / ESP32 /
   ESP8266) + inverter driver (PI30, later Deye) + JK BLE ×N; fixed pinout per
   board; wiring diagram generated alongside.
3. **Prebuilt firmware matrix** — every board × driver combination built in CI
   and published as Releases (public build repo, server stays private).
4. **Setup wizard** — «Вас приветствует Solarbox»: site, boards (what to buy,
   wiring, flash via web.esphome.io, adopt), battery, inverter, Telegram.
   Migration path for boards running site-built firmware.
5. **Installer for MikroTik** — one-line `/tool fetch` + `/import`, picks the
   image by CPU, guides through device-mode.
6. **Trusted certificate + full PWA** — HTTPS itself is done (self-signed);
   a trusted one (e.g. DNS-01 on an own domain) enables PWA install and Web
   Serial flashing straight from the wizard.
7. **Releases & versioning** — numbered server and firmware versions,
   «есть обновление» in the UI.

## Ideas / later

- Deye (Modbus) inverter driver; JK over RS485; other BMS (Daly, Seplos, Pace)
- Extra modules: temperature sensors, relays (e.g. boiler), e-ink display
- Build on demand for non-standard boards (relay on own server holding the token)
