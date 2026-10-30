//! Boards announce themselves: ingest packets may carry a `board` block
//! (type, fw, mac, ip, and driver extras such as BLE scan results). The latest
//! one per board is kept in memory for the setup wizard, which adopts unknown
//! boards into config.json.

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

impl Discovery {
    pub fn note(&self, device: &str, board: &Value, ts: i64) {
        let key = board["mac"].as_str().map(String::from).unwrap_or_else(|| device.to_string());
        let mut m = self.m.lock().unwrap();
        if m.len() > 64 && !m.contains_key(&key) {
            return; // don't let a noisy network grow this without bound
        }
        m.insert(key, Seen { device: device.to_string(), board: board.clone(), ts });
    }
}

/// GET /api/discovered — announced boards, newest first, with whether a
/// config.json board already points at the same IP.
pub async fn api(State(st): State<AppState>) -> Json<Value> {
    let cfg = st.config.get();
    let now = crate::now();
    let mut out: Vec<(i64, Value)> = st
        .discovery
        .m
        .lock()
        .unwrap()
        .values()
        .map(|s| {
            let ip = s.board["ip"].as_str().unwrap_or("");
            let adopted = cfg.boards.iter().find(|b| !ip.is_empty() && b.host == ip).map(|b| b.name.clone());
            (s.ts, json!({ "device": s.device, "ts": s.ts, "age": now - s.ts, "board": s.board, "adopted_as": adopted }))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Json(Value::Array(out.into_iter().map(|x| x.1).collect()))
}
