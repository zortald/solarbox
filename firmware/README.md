# Firmware — boards push to solarbox

Two ESPHome boards feed the server. They are **copied here as-is** for the
record; the live versions still run the standalone web panel. To make a board
*push* to solarbox, add an `http_request` client and a periodic `interval:`
that POSTs a small JSON document to `/api/ingest`.

- `inverter.yaml` — ESP8266 NodeMCU, PI30/pipsolar inverter logger.
- `jk-bms.yaml` — ESP32-C3, JK BMS over BLE (`JK02_32S`).

## Ingest contract

`POST /api/ingest`, `Content-Type: application/json`:

```json
{ "device": "bms", "metrics": { "soc": 92, "bms_v": 27.6, "bms_a": -4.1 } }
```

- `device` — `"inverter"` or `"bms"` (any string; groups the metrics).
- `metrics` — flat map of `key → number`. Keys should match the dashboard's
  `META` table in `server/static/app.js` to get nice labels/units; unknown keys
  still store and chart, just with the raw key.
- `ts` — optional unix seconds; omit and the server stamps arrival time.
- An array of such objects is also accepted for batching.
- If the server runs with `SOLARBOX_TOKEN`, send header `X-Token: <secret>`.

Booleans (charge/discharge/balance flags) go as `1` / `0`.

## ESPHome snippet to add to a board

Set the server address to wherever solarbox runs (e.g. `.230` on the boards' LAN). This sends every 10 s and does not block the existing web panel.

```yaml
http_request:
  useragent: esphome/solarbox
  timeout: 5s
  # ESP8266 needs the esp8266 variant; on ESP32 just `http_request:` is enough.

interval:
  - interval: 10s
    then:
      - http_request.post:
          url: "http://192.168.88.230:8080/api/ingest"
          headers:
            Content-Type: application/json
            # X-Token: !secret solarbox_token   # only if the server requires it
          json: |-
            root["device"] = "bms";
            root["metrics"]["soc"]        = id(soc_sensor).state;
            root["metrics"]["bms_v"]      = id(pack_voltage).state;
            root["metrics"]["bms_a"]      = id(pack_current).state;
            root["metrics"]["bms_w"]      = id(pack_power).state;
            root["metrics"]["cell_min"]   = id(cell_min).state;
            root["metrics"]["cell_max"]   = id(cell_max).state;
            root["metrics"]["cell_delta"] = id(cell_delta).state;
            root["metrics"]["chg"]        = id(charging).state ? 1 : 0;
            root["metrics"]["dsg"]        = id(discharging).state ? 1 : 0;
```

Give each sensor you want to push an `id:` in its config, then reference it in
the `json:` lambda. For the inverter board use `"device": "inverter"` and keys
like `ac_out_w`, `pv_w`, `grid_v`, `batt_v`, `load_pct`.

> Flashing is a separate, careful step over the tunnel (OTA) — do not change
> the live boards casually; they are remote. Validate the POST against a local
> `solarbox` run first.
