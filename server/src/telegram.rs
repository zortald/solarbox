//! Telegram: send messages (automation action, test button) and answer
//! /status, /help in the configured chat via long polling. Bot token and chat
//! id live in `secrets.json` on the data volume — never served by the API,
//! only replaced by the admin.

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Duration};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Tg {
    pub token: String,
    pub chat_id: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Secrets {
    #[serde(default)]
    telegram: Option<Tg>,
}

pub struct Store {
    path: PathBuf,
    cur: Mutex<Option<Tg>>,
}

impl Store {
    pub fn open(path: PathBuf) -> Self {
        let cur = std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str::<Secrets>(&s).ok())
            .and_then(|s| s.telegram)
            .filter(|t| !t.token.is_empty());
        Store { path, cur: Mutex::new(cur) }
    }
    pub fn get(&self) -> Option<Tg> {
        self.cur.lock().unwrap().clone()
    }
    pub fn set(&self, tg: Option<Tg>) -> Result<(), String> {
        let s = Secrets { telegram: tg.clone() };
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&s).unwrap())
            .and_then(|_| std::fs::rename(&tmp, &self.path))
            .map_err(|e| e.to_string())?;
        *self.cur.lock().unwrap() = tg;
        Ok(())
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(40)).build()
}

/// Blocking send (call from spawn_blocking / the bot thread).
pub fn send_blocking(tg: &Tg, text: &str) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", tg.token);
    match agent().post(&url).send_form(&[("chat_id", &tg.chat_id), ("text", text)]) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, r)) => Err(format!("Telegram HTTP {code}: {}", r.into_string().unwrap_or_default())),
        Err(e) => Err(format!("Telegram: {e}")),
    }
}

pub async fn send(st: &AppState, text: String) -> Result<(), String> {
    let Some(tg) = st.tg.get() else { return Err("Telegram не настроен".into()) };
    if tg.chat_id.is_empty() { return Err("не указан chat id (напиши боту /id)".into()) }
    tokio::task::spawn_blocking(move || send_blocking(&tg, &text)).await.map_err(|e| e.to_string())?
}

/// Replace {device.metric} with the current value (or "—").
pub fn render(st: &AppState, tpl: &str) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('}') {
            Some(b) => {
                let key = &after[..b];
                match key.split_once('.') {
                    Some((d, m)) => out.push_str(&st.autom.value(d, m).map(fmt).unwrap_or_else(|| "—".into())),
                    None => { out.push('{'); out.push_str(key); out.push('}'); }
                }
                rest = &after[b + 1..];
            }
            None => { out.push_str(&rest[a..]); rest = ""; }
        }
    }
    out.push_str(rest);
    out
}

fn fmt(v: f64) -> String {
    let a = v.abs();
    if a >= 100.0 { format!("{v:.0}") } else if a >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}

fn status_text(st: &AppState) -> String {
    let v = |d: &str, m: &str| st.autom.value(d, m);
    let f = |x: Option<f64>, unit: &str| x.map(|x| format!("{} {unit}", fmt(x))).unwrap_or_else(|| "—".into());
    let pv = v("inverter", "pv_w").unwrap_or(0.0);
    let load = v("inverter", "ac_out_w").unwrap_or(0.0);
    let batt = v("bms", "bms_w").unwrap_or(0.0);
    let gv = v("inverter", "grid_v");
    let grid_up = gv.is_some_and(|g| g > 150.0);
    let grid = if grid_up { (load + batt - pv).max(0.0) } else { 0.0 };
    format!(
        "☀ Солнце: {}\n🏠 Дом: {}\n🔋 АКБ: {} · {} ({})\n⚡ Сеть: {}\n🌡 АКБ {} · инвертор {}\n▯ Разброс ячеек: {}",
        f(Some(pv), "Вт"),
        f(Some(load), "Вт"),
        f(v("bms", "soc"), "%"),
        f(Some(batt.abs()), "Вт"),
        if batt > 5.0 { "заряд" } else if batt < -5.0 { "разряд" } else { "покой" },
        if grid_up { format!("{} В, ≈ {} Вт", fmt(gv.unwrap()), fmt(grid)) } else { "нет".into() },
        f(v("bms", "t1"), "°C"),
        f(v("inverter", "temp"), "°C"),
        v("bms", "cell_delta").map(|d| format!("{:.0} мВ", d * 1000.0)).unwrap_or_else(|| "—".into()),
    )
}

/// Long-poll loop answering commands from the configured chat only.
pub fn spawn_bot(st: AppState) {
    std::thread::spawn(move || {
        let mut offset: i64 = 0;
        loop {
            let Some(tg) = st.tg.get() else { std::thread::sleep(Duration::from_secs(30)); continue };
            let url = format!("https://api.telegram.org/bot{}/getUpdates?timeout=25&offset={offset}", tg.token);
            let resp: Result<serde_json::Value, String> = agent().get(&url).call()
                .map_err(|e| e.to_string())
                .and_then(|r| r.into_json().map_err(|e| e.to_string()));
            let upd = match resp {
                Ok(v) => v,
                Err(e) => { eprintln!("[telegram] poll: {e}"); std::thread::sleep(Duration::from_secs(30)); continue; }
            };
            for u in upd["result"].as_array().cloned().unwrap_or_default() {
                offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                let msg = &u["message"];
                let chat = msg["chat"]["id"].as_i64().map(|c| c.to_string()).unwrap_or_default();
                let text = msg["text"].as_str().unwrap_or("").trim();
                let cmd = text.split(['@', ' ']).next().unwrap_or("");
                // /id answers anyone (setup: tells them their chat id, nothing else).
                if cmd == "/id" {
                    let _ = send_blocking(&Tg { token: tg.token.clone(), chat_id: chat.clone() }, &format!("chat id: {chat}"));
                    continue;
                }
                if chat.is_empty() || chat != tg.chat_id {
                    continue; // everything else only for the configured chat
                }
                let reply = match cmd {
                    "/status" | "/s" => status_text(&st),
                    "/help" | "/start" => "/status — что сейчас с солнцем, АКБ, домом и сетью\n/id — номер этого чата".to_string(),
                    _ => continue,
                };
                if let Err(e) = send_blocking(&tg, &reply) {
                    eprintln!("[telegram] reply: {e}");
                }
            }
        }
    });
}
