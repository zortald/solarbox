#!/usr/bin/env python3
"""Generate firmware/jk-ble.yaml — the generic Solarbox board for JK BMS over
Bluetooth, with SLOTS battery slots on one ESP32 (each: own MAC, protocol and
device name, all set at runtime and kept in flash).

    python3 firmware/gen-jk-ble.py [slots]      # default 2

Slot 1 keeps the plain entity names (and device name "bms"), so a one-battery
site looks exactly like before; slot N > 1 adds " #N" to every entity name and
defaults to device "bmsN".
"""
import sys

SLOTS = int(sys.argv[1]) if len(sys.argv) > 1 else 2
CELLS = 16
FW = "1.1.0"
# BLE scan results ride along with slot 1's packet (sent even with no BMS set,
# so the wizard can list what's around).
FOUND = """                  {
                    JsonArray f = b["found"].to<JsonArray>();
                    for (auto &kv : id(found)) {
                      JsonObject o = f.add<JsonObject>();
                      o["mac"] = kv.first;
                      auto q = kv.second.rfind('|');
                      o["name"] = kv.second.substr(0, q);
                      o["rssi"] = atoi(kv.second.c_str() + q + 1);
                    }
                  }
"""


def sfx(n):
    return "" if n == 1 else f" #{n}"


def slot_block(n):
    """Per-slot YAML pieces, keyed by top-level section."""
    s, i = sfx(n), n
    cells = "".join(f"    cell_voltage_{c}:\n      id: b{i}_c{c}\n      internal: true\n" for c in range(1, CELLS + 1))
    res = "".join(f"    cell_resistance_{c}:\n      id: b{i}_r{c}\n      internal: true\n" for c in range(1, CELLS + 1))
    return {
        "ble_client": f"""  - mac_address: "00:00:00:00:00:0{i}"
    id: bms{i}_client
    auto_connect: false
""",
        "jk_bms_ble": f"""  - ble_client_id: bms{i}_client
    id: bms{i}
    protocol_version: JK02_32S
    throttle: 2s
""",
        "script": f"""  - id: apply_bms{i}
    then:
      - lambda: |-
          auto *cl = id(bms{i}_client);
          std::string pr = id(bms{i}_proto).current_option();
          id(bms{i}).set_protocol_version(pr == "JK04" ? jk_bms_ble::PROTOCOL_VERSION_JK04
              : pr == "JK02_24S" ? jk_bms_ble::PROTOCOL_VERSION_JK02_24S : jk_bms_ble::PROTOCOL_VERSION_JK02_32S);
          uint64_t a = 0; int n = 0;
          for (char c : id(bms{i}_mac).state) {{
            int v = (c >= '0' && c <= '9') ? c - '0' : (c >= 'a' && c <= 'f') ? c - 'a' + 10 : (c >= 'A' && c <= 'F') ? c - 'A' + 10 : -1;
            if (v < 0) continue;
            a = (a << 4) | v; n++;
          }}
          cl->disconnect();
          if (n != 12) {{ cl->set_auto_connect(false); ESP_LOGI("solarbox", "слот {i}: BMS не задана"); return; }}
          cl->set_address(a);
          cl->set_auto_connect(true);
          ESP_LOGI("solarbox", "слот {i}: BMS %s, %s", id(bms{i}_mac).state.c_str(), pr.c_str());
""",
        "text": f"""  - platform: template
    id: bms{i}_mac
    name: "BMS MAC{s}"
    mode: text
    optimistic: true
    restore_value: true
    max_length: 17
    initial_value: ""
    on_value:
      - script.execute: apply_bms{i}
  - platform: template
    id: dev{i}_name
    name: "Имя в Solarbox{s}"
    mode: text
    optimistic: true
    restore_value: true
    max_length: 24
    initial_value: "{'bms' if i == 1 else f'bms{i}'}"
""",
        "select": f"""  - platform: template
    id: bms{i}_proto
    name: "Протокол BMS{s}"
    options: ["JK02_32S", "JK02_24S", "JK04"]
    optimistic: true
    restore_value: true
    initial_option: "JK02_32S"
    on_value:
      - script.execute: apply_bms{i}
""",
        "sensor": f"""  - platform: jk_bms_ble
    jk_bms_ble_id: bms{i}
    total_voltage:
      id: b{i}_v
      name: "Напряжение АКБ V{s}"
    current:
      id: b{i}_a
      name: "Ток АКБ I{s}"
    power:
      id: b{i}_w
      name: "Мощность АКБ W{s}"
    state_of_charge:
      id: b{i}_soc
      name: "Заряд АКБ SOC{s}"
    capacity_remaining:
      id: b{i}_cap
      name: "Остаток АКБ Ah{s}"
    full_charge_capacity:
      id: b{i}_fcc
      name: "Ёмкость полного заряда Ah{s}"
    charging_cycles:
      id: b{i}_cycles
      name: "Циклы заряда{s}"
    temperature_sensor_1:
      id: b{i}_t1
      name: "Темп АКБ T1{s}"
    temperature_sensor_2:
      id: b{i}_t2
      name: "Темп АКБ T2{s}"
    power_tube_temperature:
      id: b{i}_tmos
      name: "Темп MOS Tm{s}"
    min_cell_voltage:
      id: b{i}_cmin
      name: "Ячейка мин Vmin{s}"
    max_cell_voltage:
      id: b{i}_cmax
      name: "Ячейка макс Vmax{s}"
    delta_cell_voltage:
      id: b{i}_cd
      name: "Разброс ячеек Vd{s}"
{cells}{res}    min_voltage_cell:
      id: b{i}_cmin_i
      internal: true
    max_voltage_cell:
      id: b{i}_cmax_i
      internal: true
    average_cell_voltage:
      id: b{i}_cavg
      internal: true
    balancing_current:
      id: b{i}_bal_a
      internal: true
""",
        "binary_sensor": f"""  - platform: jk_bms_ble
    jk_bms_ble_id: bms{i}
    charging:
      id: b{i}_chg
      name: "Заряжается CHG{s}"
    discharging:
      id: b{i}_dsg
      name: "Разряжается DSG{s}"
    balancing:
      id: b{i}_bal
      name: "Баланс BAL{s}"
""",
        "text_sensor": f"""  - platform: jk_bms_ble
    jk_bms_ble_id: bms{i}
    errors:
      name: "Ошибки ERR{s}"
    total_runtime_formatted:
      name: "Наработка RT{s}"
    device_model:
      name: "Модель BMS{s}"
    hardware_version:
      name: "Версия железа BMS{s}"
    software_version:
      name: "Версия прошивки BMS{s}"
""",
        "button": f"""  - platform: jk_bms_ble
    jk_bms_ble_id: bms{i}
    retrieve_settings:
      name: "Прочитать настройки BMS{s}"
""",
        "number": f"""  - platform: jk_bms_ble
    jk_bms_ble_id: bms{i}
    total_battery_capacity:
      name: "Ёмкость АКБ Ah{s}"
    cell_soc100_voltage:
      name: "Порог SOC 100% V{s}"
    cell_soc0_voltage:
      name: "Порог SOC 0% V{s}"
    soc_calibration:
      name: "Установить заряд SOC{s}"
""",
        "push": f"""      - if:
          condition:
            lambda: return wifi::global_wifi_component->is_connected() && ({'true' if i == 1 else f'!id(bms{i}_mac).state.empty()'});
          then:
            - http_request.post:
                url: !lambda return id(sb_target);
                request_headers:
                  Content-Type: application/json
                json: |-
                  root["device"] = id(dev{i}_name).state.empty() ? "{'bms' if i == 1 else f'bms{i}'}" : id(dev{i}_name).state;
                  JsonObject b = root["board"].to<JsonObject>();
                  b["type"] = "jk-ble";
                  b["fw"] = "${{fw_version}}";
                  b["mac"] = get_mac_address_pretty();
                  b["slot"] = {i};
                  b["slots"] = {SLOTS};
                  b["bms_mac"] = id(bms{i}_mac).state;
                  b["proto"] = id(bms{i}_proto).current_option();
                  b["connected"] = id(bms{i}_client)->connected();
                  {{
                    esp_netif_ip_info_t ii;
                    esp_netif_get_ip_info(esp_netif_get_handle_from_ifkey("WIFI_STA_DEF"), &ii);
                    char ipb[16];
                    esp_ip4addr_ntoa(&ii.ip, ipb, sizeof ipb);
                    b["ip"] = ipb;
                  }}
""" + (FOUND if i == 1 else "") + f"""                  JsonObject m = root["metrics"].to<JsonObject>();
                  auto put = [&](const char *k, float v) {{ if (!std::isnan(v)) m[k] = v; }};
                  auto flag = [&](const char *k, binary_sensor::BinarySensor *s) {{ if (s->has_state()) m[k] = s->state ? 1 : 0; }};
                  if (!id(bms{i}_client)->connected()) return;
                  put("soc", id(b{i}_soc).state);
                  put("bms_v", id(b{i}_v).state);
                  put("bms_a", id(b{i}_a).state);
                  put("bms_w", id(b{i}_w).state);
                  put("cap_ah", id(b{i}_cap).state);
                  put("fcc_ah", id(b{i}_fcc).state);
                  put("cycles", id(b{i}_cycles).state);
                  put("t1", id(b{i}_t1).state);
                  put("t2", id(b{i}_t2).state);
                  put("t_mos", id(b{i}_tmos).state);
                  put("cell_min", id(b{i}_cmin).state);
                  put("cell_max", id(b{i}_cmax).state);
                  put("cell_delta", id(b{i}_cd).state);
                  flag("chg", id(b{i}_chg));
                  flag("dsg", id(b{i}_dsg));
                  flag("bal", id(b{i}_bal));
                  put("rssi", id(w_rssi).state);
                  put("uptime", id(up).state);
                  put("cell_avg", id(b{i}_cavg).state);
                  put("bal_a", id(b{i}_bal_a).state);
                  put("cell_min_i", id(b{i}_cmin_i).state);
                  put("cell_max_i", id(b{i}_cmax_i).state);
                  sensor::Sensor *const cv[] = {{{", ".join(f"id(b{i}_c{c})" for c in range(1, CELLS + 1))}}};
                  sensor::Sensor *const cr[] = {{{", ".join(f"id(b{i}_r{c})" for c in range(1, CELLS + 1))}}};
                  char k[5];
                  for (int c = 0; c < {CELLS}; c++) {{
                    snprintf(k, sizeof k, "c%d", c + 1); put(k, cv[c]->state);
                    snprintf(k, sizeof k, "r%d", c + 1); put(k, cr[c]->state);
                  }}
""",
    }


def main():
    slots = [slot_block(n) for n in range(1, SLOTS + 1)]
    sec = lambda name: "".join(s[name] for s in slots)
    applies = "".join(f"        - script.execute: apply_bms{n}\n" for n in range(1, SLOTS + 1))
    y = f"""# GENERATED by gen-jk-ble.py ({SLOTS} slots) — edit the generator, not this file.
#
# Solarbox · плата JK BMS по Bluetooth (ESP32-C3), до {SLOTS} BMS на одной плате.
# Универсальная прошивка: ничего про объект внутри. Wi-Fi задаётся после прошивки
# (web.esphome.io → Improv, или точка доступа платы), адрес Solarbox, MAC и
# протокол каждой BMS — в мастере Solarbox или на веб-странице платы. Всё хранится
# во флеше и переживает обновления по воздуху.
#
# Пустой «Solarbox URL» = http://<шлюз сети>:8080 — Solarbox на роутере.
# Слот без MAC ничего не делает и ничего не шлёт.

substitutions:
  fw_version: "{FW}"

esphome:
  name: solarbox-bms
  friendly_name: Solarbox BMS
  name_add_mac_suffix: true
  project:
    name: solarbox.jk-ble
    version: ${{fw_version}}
  on_boot:
    - priority: -100
      then:
{applies}
esp32:
  board: esp32-c3-devkitm-1
  framework:
    type: esp-idf

logger:

ota:
  - platform: esphome
  - platform: web_server

# Сетей нет в прошивке — поэтому сохранённый Wi-Fi не теряется при обновлениях.
wifi:
  ap:
    ssid: "Solarbox-BMS"
    password: "solarbox"
  power_save_mode: none

captive_portal:
improv_serial:

web_server:
  port: 80

external_components:
  - source: github://syssi/esphome-jk-bms@main
    components: [jk_bms_ble]
    refresh: 1d

# Поиск BMS рядом: имена JK-* попадают в список, Solarbox показывает его в мастере.
globals:
  - id: found
    type: std::map<std::string, std::string>
  - id: sb_target
    type: std::string

esp32_ble:
  max_connections: {max(3, SLOTS)}

esp32_ble_tracker:
  scan_parameters:
    active: true
  on_ble_advertise:
    - then:
        - lambda: |-
            const std::string &n = x.get_name();
            if (n.rfind("JK", 0) == 0 || n.rfind("jk", 0) == 0) {{
              if (id(found).size() < 12 || id(found).count(x.address_str()))
                id(found)[x.address_str()] = n + "|" + std::to_string(x.get_rssi());
            }}

ble_client:
{sec("ble_client")}
jk_bms_ble:
{sec("jk_bms_ble")}
script:
  - id: ingest_url
    then:
      - lambda: |-
          std::string u = id(sb_url).state;
          if (u.empty()) {{
            esp_netif_ip_info_t ip;
            esp_netif_get_ip_info(esp_netif_get_handle_from_ifkey("WIFI_STA_DEF"), &ip);
            char gw[16];
            esp_ip4addr_ntoa(&ip.gw, gw, sizeof gw);
            u = std::string("http://") + gw + ":8080";
          }}
          while (!u.empty() && u.back() == '/') u.pop_back();
          id(sb_target) = u + "/api/ingest";
{sec("script")}
text:
  - platform: template
    id: sb_url
    name: "Solarbox URL"
    mode: text
    optimistic: true
    restore_value: true
    max_length: 96
    initial_value: ""
{sec("text")}
select:
{sec("select")}
sensor:
  - platform: wifi_signal
    id: w_rssi
    name: "Сигнал Wi-Fi"
  - platform: uptime
    id: up
    internal: true
    update_interval: 60s
{sec("sensor")}
binary_sensor:
{sec("binary_sensor")}
text_sensor:
  - platform: version
    name: "Версия FW"
{sec("text_sensor")}
button:
  - platform: restart
    name: "Перезагрузка RST"
  - platform: template
    name: "Искать BMS"
    on_press:
      - lambda: id(found).clear();
{sec("button")}
number:
{sec("number")}
# Отправка в Solarbox: по запросу на каждую заданную BMS (слот 1 — всегда,
# с данными о плате и найденных рядом BMS).
http_request:
  verify_ssl: false
  timeout: 3s

interval:
  - interval: 5s
    then:
      - script.execute: ingest_url
{sec("push")}"""
    open(__file__.replace("gen-jk-ble.py", "jk-ble.yaml"), "w").write(y)
    print(f"jk-ble.yaml: {SLOTS} slots, {CELLS} cells each")


main()
