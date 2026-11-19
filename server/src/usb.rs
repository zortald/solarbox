//! Boards plugged into the router's USB. The installer passes the router's
//! whole USB controller into the container, so every board shows up in
//! /sys/class/tty; its /dev node, though, exists only for boards that were
//! there when the container started (RouterOS makes the nodes, a container
//! may not mknod). A port without a node → Solarbox restarts itself
//! (restart-policy brings it back ≈10 s later with the node).
//! espflash talks to the boards: chip and MAC, and the first flash.

use crate::AppState;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path, sync::Mutex, time::Duration};

#[derive(Default)]
pub struct Usb {
    /// Port name → what espflash read from the board (chip, MAC…).
    known: Mutex<HashMap<String, Value>>,
    /// Port name → flashing progress / result for the UI.
    pub job: Mutex<HashMap<String, Value>>,
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// What a USB id usually is on Solarbox boards.
pub fn kind(vid: &str, pid: &str) -> &'static str {
    match (vid, pid) {
        ("303a", "1001") => "ESP32-C3/S3 (встроенный USB)",
        ("1a86", "7523") | ("1a86", "55d4") | ("1a86", "55d3") => "плата с CH340/CH9102 (ESP32 DevKit, D1 mini…)",
        ("10c4", "ea60") => "плата с CP2102",
        ("0403", "6001") | ("0403", "6015") => "плата с FTDI",
        _ => "неизвестное USB-устройство",
    }
}

/// Serial ports the kernel sees, with their USB identity.
pub fn ports() -> Vec<Value> {
    let mut out = Vec::new();
    for e in std::fs::read_dir("/sys/class/tty").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("ttyACM") || name.starts_with("ttyUSB")) {
            continue;
        }
        // …/ttyACM0/device → the USB interface; its parent (ACM) or
        // grandparent (usb-serial: …/ttyUSB0/device is the port) is the device.
        let dev = std::fs::canonicalize(e.path().join("device")).unwrap_or_default();
        let usb = dev.ancestors().find(|p| p.join("idVendor").exists()).map(Path::to_path_buf).unwrap_or_default();
        let (vid, pid) = (read(&usb.join("idVendor")), read(&usb.join("idProduct")));
        out.push(json!({
            "port": name, "node": Path::new("/dev").join(&name).exists(),
            "vid": vid, "pid": pid, "kind": kind(&vid, &pid),
            "manufacturer": read(&usb.join("manufacturer")), "product": read(&usb.join("product")),
            "serial": read(&usb.join("serial")), "usb": usb.file_name().map(|n| n.to_string_lossy().to_string()),
        }));
    }
    out.sort_by(|a, b| a["port"].as_str().cmp(&b["port"].as_str()));
    out
}

fn connect(port: &str, vid: u16, pid: u16) -> Result<espflash::flasher::Flasher, String> {
    use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
    let serial = serialport::new(format!("/dev/{port}"), 115_200).timeout(Duration::from_secs(3)).open_native().map_err(|e| format!("порт {port}: {e}"))?;
    let info = serialport::UsbPortInfo { vid, pid, serial_number: None, manufacturer: None, product: None };
    let conn = Connection::new(serial, info, ResetAfterOperation::HardReset, ResetBeforeOperation::DefaultReset, 115_200);
    espflash::flasher::Flasher::connect(conn, true, true, false, None, None).map_err(|e| format!("плата не отвечает загрузчиком: {e}"))
}

fn hex(s: &str) -> u16 {
    u16::from_str_radix(s, 16).unwrap_or(0)
}

/// Read chip and MAC (the board resets into its bootloader and back).
pub fn identify(port: &str, vid: &str, pid: &str) -> Result<Value, String> {
    let mut f = connect(port, hex(vid), hex(pid)).map_err(|e| {
        // espflash knows only the ESP32 family; an ESP8266 answers its ROM
        // loader with a chip id it doesn't recognise.
        if e.to_lowercase().contains("chip") || e.contains("8266") {
            "чип не из семейства ESP32 (ESP8266?) — не поддерживается, нужна ESP32-C3/S3/ESP32".to_string()
        } else {
            e
        }
    })?;
    let d = f.device_info().map_err(|e| e.to_string())?;
    Ok(json!({ "chip": d.chip.to_string(), "mac": d.mac_address.unwrap_or_default().to_ascii_uppercase(),
               "flash": d.flash_size.to_string(), "features": d.features }))
}

struct Progress<'a> {
    usb: &'a Usb,
    port: String,
    total: usize,
}

impl espflash::target::ProgressCallbacks for Progress<'_> {
    fn init(&mut self, _addr: u32, total: usize) {
        self.total = total.max(1);
    }
    fn update(&mut self, current: usize) {
        self.usb.job.lock().unwrap().insert(self.port.clone(), json!({ "state": "flashing", "pct": current * 100 / self.total }));
    }
    fn verifying(&mut self) {
        self.usb.job.lock().unwrap().insert(self.port.clone(), json!({ "state": "verifying", "pct": 100 }));
    }
    fn finish(&mut self, _skipped: bool) {}
}

/// Write a factory image (bootloader + partitions + app) at 0.
pub fn flash(usb: &Usb, port: &str, vid: &str, pid: &str, image: &[u8]) -> Result<(), String> {
    let mut f = connect(port, hex(vid), hex(pid))?;
    let mut p = Progress { usb, port: port.to_string(), total: image.len() };
    f.write_bin_to_flash(0, image, &mut p).map_err(|e| e.to_string())
}

// ---- Improv: Wi-Fi over the same cable --------------------------------------

fn improv_packet(typ: u8, data: &[u8]) -> Vec<u8> {
    let mut p = b"IMPROV".to_vec();
    p.extend_from_slice(&[1, typ, data.len() as u8]);
    p.extend_from_slice(data);
    let sum = p.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    p.push(sum);
    p.push(b'\n');
    p
}

/// Packets (type, data) found in what the board printed (log lines too).
fn improv_parse(buf: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(k) = buf[i..].windows(6).position(|w| w == b"IMPROV") {
        let s = i + k;
        let Some(&n) = buf.get(s + 8) else { break };
        let end = s + 9 + n as usize;
        if buf.len() <= end {
            break;
        }
        out.push((buf[s + 7], buf[s + 9..end].to_vec()));
        i = end + 1;
    }
    out
}

/// Give the freshly flashed board its Wi-Fi (Improv Serial, as
/// web.esphome.io does). Returns the URL the board reports.
pub fn wifi(port: &str, ssid: &str, pw: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    if ssid.is_empty() || ssid.len() > 32 || pw.len() > 63 {
        return Err("сеть или пароль не той длины".into());
    }
    let mut sp = serialport::new(format!("/dev/{port}"), 115_200).timeout(Duration::from_millis(300)).open_native().map_err(|e| format!("порт {port}: {e}"))?;
    let mut payload = vec![ssid.len() as u8];
    payload.extend_from_slice(ssid.as_bytes());
    payload.push(pw.len() as u8);
    payload.extend_from_slice(pw.as_bytes());
    let mut rpc = vec![1u8, payload.len() as u8];
    rpc.extend_from_slice(&payload);
    // The board may still be booting after the flash: ask until it answers.
    let start = std::time::Instant::now();
    let (mut buf, mut sent, mut chunk) = (Vec::new(), None::<std::time::Instant>, [0u8; 512]);
    while start.elapsed() < Duration::from_secs(60) {
        if sent.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) {
            sp.write_all(&improv_packet(3, &rpc)).map_err(|e| e.to_string())?;
            sent = Some(std::time::Instant::now());
        }
        if let Ok(n) = sp.read(&mut chunk) {
            buf.extend_from_slice(&chunk[..n]);
        }
        for (typ, d) in improv_parse(&buf) {
            match typ {
                // error state: 3 = unable to connect
                2 if d.first().is_some_and(|e| *e != 0) => {
                    return Err(if d[0] == 3 { "плата не смогла подключиться к этой сети — проверьте пароль".into() } else { format!("плата ответила ошибкой {}", d[0]) });
                }
                // RPC result: the board's URL(s)
                4 => {
                    let url = d.get(3..3 + *d.get(2).unwrap_or(&0) as usize).map(|u| String::from_utf8_lossy(u).to_string()).unwrap_or_default();
                    return Ok(url);
                }
                _ => {}
            }
        }
        if buf.len() > 65536 {
            buf.drain(..buf.len() - 4096);
        }
    }
    Err("плата не ответила по Improv за минуту".into())
}

/// Networks the board itself sees (Improv RPC 4): (ssid, rssi, secured),
/// strongest first.
pub fn scan(port: &str) -> Result<Vec<(String, i32, bool)>, String> {
    use std::io::{Read, Write};
    let mut sp = serialport::new(format!("/dev/{port}"), 115_200).timeout(Duration::from_millis(300)).open_native().map_err(|e| format!("порт {port}: {e}"))?;
    let start = std::time::Instant::now();
    let (mut buf, mut sent, mut chunk) = (Vec::new(), None::<std::time::Instant>, [0u8; 512]);
    while start.elapsed() < Duration::from_secs(30) {
        // The board may still be booting after the flash: ask until it answers.
        if sent.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) {
            buf.clear();
            sp.write_all(&improv_packet(3, &[4, 0])).map_err(|e| e.to_string())?;
            sent = Some(std::time::Instant::now());
        }
        if let Ok(n) = sp.read(&mut chunk) {
            buf.extend_from_slice(&chunk[..n]);
        }
        let results: Vec<Vec<u8>> = improv_parse(&buf).into_iter().filter(|(t, d)| *t == 4 && d.first() == Some(&4)).map(|(_, d)| d).collect();
        // The list ends with a result that carries no strings.
        if results.last().is_some_and(|d| d.get(1) == Some(&0)) {
            let mut out: Vec<(String, i32, bool)> = Vec::new();
            for d in &results {
                let mut strs = Vec::new();
                let mut i = 2;
                while i < d.len() {
                    let n = d[i] as usize;
                    strs.push(String::from_utf8_lossy(d.get(i + 1..i + 1 + n).unwrap_or(&[])).to_string());
                    i += 1 + n;
                }
                if let [ssid, rssi, auth, ..] = strs.as_slice() {
                    if !ssid.is_empty() && !out.iter().any(|x| &x.0 == ssid) {
                        out.push((ssid.clone(), rssi.parse().unwrap_or(-100), auth == "YES"));
                    }
                }
            }
            out.sort_by(|a, b| b.1.cmp(&a.1));
            return Ok(out);
        }
    }
    Err("плата не ответила списком сетей".into())
}

// ---- handlers ---------------------------------------------------------------

fn bad(c: StatusCode, e: impl Into<String>) -> Response {
    (c, Json(json!({ "ok": false, "error": e.into() }))).into_response()
}

/// GET /api/usb — boards in the router's USB: identity, what we know, jobs,
/// and whether a restart is needed to reach a newly plugged one.
pub async fn list(State(st): State<AppState>) -> Json<Value> {
    let known = st.usb.known.lock().unwrap().clone();
    let jobs = st.usb.job.lock().unwrap().clone();
    let disc = st.discovery.boards(&st.config.get(), crate::now());
    let mut ports = ports();
    for p in &mut ports {
        let name = p["port"].as_str().unwrap_or("").to_string();
        if let Some(k) = known.get(&name) {
            p["board"] = k.clone();
        }
        if let Some(j) = jobs.get(&name) {
            p["job"] = j.clone();
        }
        // Already running Solarbox and talking to us: its MAC (the built-in
        // USB's serial number, or what espflash read) is among the boards seen.
        let mac = p["board"]["mac"].as_str().filter(|m| !m.is_empty()).or(p["serial"].as_str()).unwrap_or("").to_ascii_uppercase();
        if let Some(d) = disc.iter().find(|d| !mac.is_empty() && d["mac"] == mac.as_str()) {
            p["solarbox"] = json!({ "fw": d["fw"], "ip": d["ip"], "age": d["age"], "types": d["types"], "adopted_as": d["adopted_as"] });
        }
        p["tested"] = json!(p["vid"] == "303a"); // only built-in USB (C3/S3) checked on hardware so far
    }
    let restart = ports.iter().any(|p| p["node"] == false);
    Json(json!({ "ports": ports, "restart_needed": restart }))
}

#[derive(serde::Deserialize)]
pub struct PortBody {
    port: String,
}

fn find(port: &str) -> Option<Value> {
    ports().into_iter().find(|p| p["port"] == port)
}

/// POST /api/usb/identify {port}
pub async fn identify_api(State(st): State<AppState>, Json(b): Json<PortBody>) -> Response {
    let Some(p) = find(&b.port) else { return bad(StatusCode::NOT_FOUND, "нет такого порта") };
    if p["node"] == false {
        return bad(StatusCode::CONFLICT, "плату воткнули после запуска Solarbox — нужен перезапуск");
    }
    let (port, vid, pid) = (b.port.clone(), p["vid"].as_str().unwrap_or("").to_string(), p["pid"].as_str().unwrap_or("").to_string());
    match tokio::task::spawn_blocking(move || identify(&port, &vid, &pid)).await {
        Ok(Ok(v)) => {
            eprintln!("[usb] {}: {} {}", b.port, v["chip"], v["mac"]);
            st.usb.known.lock().unwrap().insert(b.port.clone(), v.clone());
            Json(json!({ "ok": true, "board": v })).into_response()
        }
        Ok(Err(e)) => bad(StatusCode::BAD_GATEWAY, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /api/usb/restart — exit so that the router restarts us with device
/// nodes for the boards plugged in since (restart-policy on-failure).
pub async fn restart() -> Response {
    eprintln!("[usb] restarting to reach newly plugged boards");
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::process::exit(75);
    });
    Json(json!({ "ok": true })).into_response()
}

#[derive(serde::Deserialize)]
pub struct FlashBody {
    port: String,
    /// Library image, e.g. "s3-jk2".
    target: String,
}

/// POST /api/usb/flash {port, target} — signed factory image from the
/// library onto the board. Progress in GET /api/usb (job).
pub async fn flash_api(State(st): State<AppState>, Json(b): Json<FlashBody>) -> Response {
    let Some(p) = find(&b.port) else { return bad(StatusCode::NOT_FOUND, "нет такого порта") };
    if p["node"] == false {
        return bad(StatusCode::CONFLICT, "плату воткнули после запуска Solarbox — нужен перезапуск");
    }
    if st.usb.job.lock().unwrap().values().any(|j| matches!(j["state"].as_str(), Some("fetching" | "flashing" | "verifying"))) {
        return bad(StatusCode::CONFLICT, "уже идёт прошивка");
    }
    st.usb.job.lock().unwrap().insert(b.port.clone(), json!({ "state": "fetching" }));
    let (st2, port, target) = (st.clone(), b.port.clone(), b.target.clone());
    let (vid, pid) = (p["vid"].as_str().unwrap_or("").to_string(), p["pid"].as_str().unwrap_or("").to_string());
    tokio::task::spawn_blocking(move || {
        let r = crate::library::factory(&target).and_then(|(ver, img)| flash(&st2.usb, &port, &vid, &pid, &img).map(|_| ver));
        let v = match &r {
            Ok(ver) => json!({ "state": "done", "target": target, "version": ver }),
            Err(e) if e == "building" => json!({ "state": "building", "target": target }),
            Err(e) => json!({ "state": "failed", "error": e }),
        };
        eprintln!("[usb] {port}: flash {target}: {v}");
        st2.usb.job.lock().unwrap().insert(port, v);
    });
    Json(json!({ "ok": true })).into_response()
}

/// POST /api/usb/scan {port} — Wi-Fi networks around the board.
pub async fn scan_api(Json(b): Json<PortBody>) -> Response {
    match tokio::task::spawn_blocking(move || scan(&b.port)).await {
        Ok(Ok(v)) => Json(json!({ "ok": true, "networks": v.iter().map(|(s, r, sec)| json!({ "ssid": s, "rssi": r, "secured": sec })).collect::<Vec<_>>() })).into_response(),
        Ok(Err(e)) => bad(StatusCode::BAD_GATEWAY, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
pub struct WifiBody {
    port: String,
    ssid: String,
    password: String,
    /// This Solarbox as the board should reach it, e.g. http://192.168.88.1:8080
    /// (written into the board: Solarbox is not always the board's gateway).
    #[serde(default)]
    solarbox: Option<String>,
}

/// POST /api/usb/wifi {port, ssid, password, solarbox} — the password goes to
/// the board only, Solarbox doesn't keep it.
pub async fn wifi_api(Json(b): Json<WifiBody>) -> Response {
    let sb = b.solarbox.clone();
    match tokio::task::spawn_blocking(move || wifi(&b.port, &b.ssid, &b.password)).await {
        Ok(Ok(url)) => {
            // http://10.10.0.120:80 → tell the board where Solarbox is.
            let host = url.split("://").nth(1).and_then(|h| h.split([':', '/']).next()).unwrap_or("").to_string();
            let mut told = None;
            if let (false, Some(sb)) = (host.is_empty(), sb) {
                let board = crate::boards::Board { name: host.clone(), host: host.clone(), web_port: 80, ota_port: 3232 };
                let path = format!("/text/{}/set?value={}", crate::boards::pct("Solarbox URL"), crate::boards::pct(&sb));
                for _ in 0..10 {
                    // its web server comes up a moment after Wi-Fi
                    if let Ok((c, _)) = crate::boards::http(&board, "POST", &path).await {
                        told = Some(c < 300);
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
            Json(json!({ "ok": true, "url": url, "solarbox_set": told })).into_response()
        }
        Ok(Err(e)) => bad(StatusCode::BAD_GATEWAY, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn improv_roundtrip() {
        let mut log = b"[I][wifi:123]: hello\n".to_vec();
        let mut p = super::improv_packet(4, &[1, 22, 21]);
        p.splice(10..10, b"http://10.10.0.120:80".to_vec()); // RPC result with one URL
        p[8] = 3 + 21;
        log.extend_from_slice(&p);
        let got = super::improv_parse(&log);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 4);
    }

    #[test]
    fn kinds() {
        assert!(super::kind("303a", "1001").contains("C3/S3"));
        assert!(super::kind("1a86", "7523").contains("CH340"));
    }
}
