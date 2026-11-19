//! Boards announce themselves: ingest packets may carry a `board` block
//! (type, fw, mac, ip, and driver extras such as BLE scan results). The latest
//! one per board slot is kept in memory for the setup wizard, which adopts
//! unknown boards into config.json. A board with several slots (one per BMS)
//! sends one packet per slot; they are listed together under its MAC.

use crate::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex};

struct Seen {
    device: String,
    board: Value,
    ts: i64,
}

#[derive(Default)]
pub struct Discovery {
    m: Mutex<HashMap<String, Seen>>,
}

fn mac(board: &Value) -> Option<String> {
    board["mac"].as_str().filter(|m| !m.is_empty()).map(|m| m.to_ascii_uppercase())
}

impl Discovery {
    pub fn note(&self, device: &str, board: &Value, ts: i64) {
        let slot = board["slot"].as_u64().unwrap_or(1);
        let key = match mac(board) {
            Some(m) => format!("{m}#{slot}"),
            None => device.to_string(),
        };
        let mut m = self.m.lock().unwrap();
        if m.len() > 64 && !m.contains_key(&key) {
            return; // don't let a noisy network grow this without bound
        }
        m.insert(key, Seen { device: device.to_string(), board: board.clone(), ts });
    }

    /// Announced boards, slots merged, newest first.
    pub fn boards(&self, cfg: &crate::config::Config, now: i64) -> Vec<Value> {
        let m = self.m.lock().unwrap();
        let mut by: HashMap<String, Vec<&Seen>> = HashMap::new();
        for s in m.values() {
            by.entry(mac(&s.board).unwrap_or_else(|| s.device.clone())).or_default().push(s);
        }
        let mut out: Vec<(i64, Value)> = by
            .into_iter()
            .map(|(key, mut seen)| {
                seen.sort_by_key(|s| s.board["slot"].as_u64().unwrap_or(1));
                let newest = seen.iter().max_by_key(|s| s.ts).unwrap();
                let b = &newest.board;
                let ip = b["ip"].as_str().unwrap_or("");
                // Adopted: same MAC in config.json, else (older entries) same IP.
                let adopted = cfg.board_by_mac(&key).or_else(|| cfg.boards.iter().find(|c| c.mac.is_none() && !ip.is_empty() && c.host == ip));
                let slots: Vec<Value> = seen
                    .iter()
                    .map(|s| json!({ "slot": s.board["slot"].as_u64().unwrap_or(1), "device": s.device,
                                     "bms_mac": s.board["bms_mac"], "proto": s.board["proto"], "connected": s.board["connected"], "age": now - s.ts }))
                    .collect();
                // A board running several drivers (pi30 + jk-ble) sends packets of each type.
                let mut types: Vec<String> = seen.iter().filter_map(|s| s.board["type"].as_str().map(String::from)).collect();
                types.sort();
                types.dedup();
                // BLE scan results ride on one slot's packet; take whichever has them.
                let found = seen.iter().find_map(|s| s.board.get("found").filter(|f| f.is_array())).cloned().unwrap_or(json!([]));
                let v = json!({ "mac": key, "type": b["type"], "types": types, "fw": b["fw"], "ip": ip, "chip": b["chip"], "slots_total": b["slots"],
                                "age": now - newest.ts, "slots": slots, "found": found, "adopted_as": adopted.map(|c| c.name.clone()) });
                (newest.ts, v)
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().map(|x| x.1).collect()
    }
}

/// An adopted board pushing from a new IP: follow it in config.json.
pub fn follow_ip(st: &AppState, board: &Value) {
    let (Some(m), Some(ip)) = (mac(board), board["ip"].as_str().filter(|i| !i.is_empty())) else { return };
    let cfg = st.config.get();
    let Some(b) = cfg.board_by_mac(&m) else { return };
    if b.host == ip {
        return;
    }
    let (name, old) = (b.name.clone(), b.host.clone());
    match st.config.update(|c| {
        if let Some(b) = c.boards.iter_mut().find(|b| b.name == name) {
            b.host = ip.to_string();
        }
    }) {
        Ok(_) => eprintln!("[discovery] {name}: new IP {old} → {ip}"),
        Err(e) => eprintln!("[discovery] {name}: new IP {ip} not saved: {e}"),
    }
}

/// GET /api/discovered — announced boards (slots merged), newest first, with
/// the config.json board each one is adopted as.
pub async fn api(State(st): State<AppState>) -> Json<Value> {
    Json(Value::Array(st.discovery.boards(&st.config.get(), crate::now())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_slots_one_board() {
        let d = Discovery::default();
        let cfg = crate::config::Config::default_with("");
        let b1 = json!({ "type": "jk-ble", "mac": "aa:bb:cc:00:00:01", "slot": 1, "ip": "10.0.0.5", "bms_mac": "", "found": [{ "mac": "C8:00", "name": "JK_B2A8S20P", "rssi": -60 }] });
        let b2 = json!({ "type": "jk-ble", "mac": "aa:bb:cc:00:00:01", "slot": 2, "ip": "10.0.0.5", "bms_mac": "C8:01" });
        d.note("bms", &b1, 100);
        d.note("bms2", &b2, 101);
        d.note("bms2", &b2, 102); // repeats replace, never pile up
        let v = d.boards(&cfg, 110);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["mac"], "AA:BB:CC:00:00:01");
        assert_eq!(v[0]["slots"].as_array().unwrap().len(), 2);
        assert_eq!(v[0]["slots"][1]["device"], "bms2");
        assert_eq!(v[0]["found"][0]["name"], "JK_B2A8S20P"); // kept although slot 2 is newer
        assert_eq!(v[0]["age"], 8);
    }
}
