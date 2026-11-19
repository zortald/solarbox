//! Telegram: bot token + a list of allowed chats (people) in `secrets.json`
//! on the data volume — never served by the API, only managed by the admin.
//! Messages (automation action, test) go to every allowed chat; /status and
//! /help answer allowed chats only. Anyone else who writes /start or /id is
//! told their request went to the owner and appears in «pending» for the admin
//! to add. Saving a token validates it (getMe) and registers the bot's command
//! menu (setMyCommands), so nothing has to be done in BotFather.

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Chat {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Tg {
    pub token: String,
    #[serde(default)]
    pub chats: Vec<Chat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<String>,
    /// Mirror every settings change to the chats (on unless switched off).
    #[serde(default = "yes")]
    pub notify_changes: bool,
    /// Logins, failed-login series, access opened/closed, public IP changes.
    #[serde(default = "yes")]
    pub notify_access: bool,
    /// Legacy single-chat field (migrated into `chats` on load).
    #[serde(default, skip_serializing)]
    chat_id: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
struct Secrets {
    #[serde(default)]
    telegram: Option<Tg>,
}

pub struct Store {
    path: PathBuf,
    cur: Mutex<Option<Tg>>,
    pending: Mutex<Vec<Chat>>,
}

impl Store {
    pub fn open(path: PathBuf) -> Self {
        let mut cur = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Secrets>(&s).ok())
            .and_then(|s| s.telegram)
            .filter(|t| !t.token.is_empty());
        let mut migrated = false;
        if let Some(t) = cur.as_mut() {
            if let Some(id) = t.chat_id.take().filter(|c| !c.is_empty()) {
                if !t.chats.iter().any(|c| c.id == id) {
                    t.chats.push(Chat { id, name: String::new(), username: None });
                }
                migrated = true;
            }
        }
        let st = Store { path, cur: Mutex::new(cur.clone()), pending: Mutex::new(vec![]) };
        if migrated {
            let _ = st.set(cur); // persist the migrated form
        }
        st
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
    pub fn pending(&self) -> Vec<Chat> {
        self.pending.lock().unwrap().clone()
    }
    fn add_pending(&self, c: Chat) {
        let mut p = self.pending.lock().unwrap();
        p.retain(|x| x.id != c.id);
        p.insert(0, c);
        p.truncate(20);
    }
    pub fn take_pending(&self, id: &str) -> Option<Chat> {
        let mut p = self.pending.lock().unwrap();
        let i = p.iter().position(|c| c.id == id)?;
        Some(p.remove(i))
    }
    /// Update stored name/username of an allowed chat if Telegram reports new ones.
    fn refresh_chat(&self, c: &Chat) {
        let mut changed = None;
        if let Some(t) = self.cur.lock().unwrap().as_mut() {
            if let Some(x) = t.chats.iter_mut().find(|x| x.id == c.id) {
                if x.name != c.name || x.username != c.username {
                    x.name = c.name.clone();
                    x.username = c.username.clone();
                    changed = Some(t.clone());
                }
            }
        }
        if let Some(t) = changed {
            let _ = self.set(Some(t));
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(40)).build()
}

fn api_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => format!("Telegram HTTP {code}: {}", r.into_string().unwrap_or_default()),
        e => format!("Telegram: {e}"),
    }
}

fn send_one(token: &str, chat: &str, text: &str) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    agent().post(&url).send_form(&[("chat_id", chat), ("text", text)]).map(|_| ()).map_err(api_err)
}

/// Validate a token and register the bot's command menu. Returns @username.
pub fn setup_bot(token: &str) -> Result<String, String> {
    let me: serde_json::Value = agent()
        .get(&format!("https://api.telegram.org/bot{token}/getMe"))
        .call()
        .map_err(api_err)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let username = me["result"]["username"].as_str().unwrap_or("").to_string();
    let commands = serde_json::json!({ "commands": [
        { "command": "status", "description": "Что сейчас: солнце, АКБ, дом, сеть, за сегодня" },
        { "command": "help", "description": "Что умеет бот" },
        { "command": "id", "description": "Номер этого чата / запросить доступ" },
    ]});
    agent()
        .post(&format!("https://api.telegram.org/bot{token}/setMyCommands"))
        .send_json(commands)
        .map_err(api_err)?;
    Ok(username)
}

/// Name/username of a chat by id (for manual adds).
pub fn chat_info(token: &str, id: &str) -> Option<Chat> {
    let v: serde_json::Value = agent()
        .get(&format!("https://api.telegram.org/bot{token}/getChat?chat_id={id}"))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    Some(chat_from(&v["result"], id))
}

fn chat_from(c: &serde_json::Value, id: &str) -> Chat {
    let name = c["title"].as_str().map(String::from).unwrap_or_else(|| {
        [c["first_name"].as_str(), c["last_name"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" ")
    });
    Chat { id: id.to_string(), name, username: c["username"].as_str().map(String::from) }
}

/// Send to every allowed chat. Ok if at least one delivery succeeded.
pub async fn send(st: &AppState, text: String) -> Result<(), String> {
    let Some(tg) = st.tg.get() else { return Err("Telegram не настроен".into()) };
    if tg.chats.is_empty() {
        return Err("нет получателей — добавь людей в Настройки → Telegram".into());
    }
    tokio::task::spawn_blocking(move || {
        let mut errs = Vec::new();
        let mut ok = 0;
        for c in &tg.chats {
            match send_one(&tg.token, &c.id, &text) {
                Ok(_) => ok += 1,
                Err(e) => errs.push(format!("{}: {e}", if c.name.is_empty() { &c.id } else { &c.name })),
            }
        }
        if ok > 0 { Ok(()) } else { Err(errs.join("; ")) }
    })
    .await
    .map_err(|e| e.to_string())?
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
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push_str(&rest[a..]);
                rest = "";
            }
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
    let g = load + batt - pv;
    let grid = if grid_up && g > 40.0 { g } else { 0.0 };
    let kwh = |k: &str| v("energy", &format!("{k}_today")).map(|x| format!("{x:.1}")).unwrap_or_else(|| "—".into());
    let outage = st
        .outages
        .current_start()
        .map(|s| {
            let m = (crate::now() - s) / 60;
            format!("\n⚠ Света нет уже {} ч {} мин", m / 60, m % 60)
        })
        .unwrap_or_default();
    format!(
        "☀ Солнце: {}\n🏠 Дом: {}\n🔋 АКБ: {} · {} ({})\n⚡ Сеть: {}{}\n🌡 АКБ {} · инвертор {}\n▯ Разброс ячеек: {}\n\nСегодня, кВт·ч: ☀ {} · 🏠 {} · ⚡ сеть ≈{} · 🔋 +{} / −{}",
        f(Some(pv), "Вт"),
        f(Some(load), "Вт"),
        f(v("bms", "soc"), "%"),
        f(Some(batt.abs()), "Вт"),
        if batt > 5.0 { "заряд" } else if batt < -5.0 { "разряд" } else { "покой" },
        if grid_up { format!("{} В, ≈ {} Вт", fmt(gv.unwrap()), fmt(grid)) } else { "нет".into() },
        outage,
        f(v("bms", "t1"), "°C"),
        f(v("inverter", "temp"), "°C"),
        v("bms", "cell_delta").map(|d| format!("{:.0} мВ", d * 1000.0)).unwrap_or_else(|| "—".into()),
        kwh("pv"),
        kwh("load"),
        kwh("grid"),
        kwh("chg"),
        kwh("dis"),
    )
}

const HELP: &str = "/status — что сейчас: солнце, АКБ, дом, сеть и итоги за сегодня\n/id — номер этого чата\n\nУведомления приходят по правилам из вкладки «Автоматизации» дашборда.";

/// Long-poll loop. Allowed chats get answers; others can only ask for access.
pub fn spawn_bot(st: AppState) {
    std::thread::spawn(move || {
        // Register the command menu, remember the bot's @name and fill in
        // names of recipients that were added by bare chat id.
        if let Some(mut t) = st.tg.get() {
            match setup_bot(&t.token) {
                Ok(bot) => t.bot = Some(bot),
                Err(e) => eprintln!("[telegram] setup: {e}"),
            }
            for c in t.chats.iter_mut().filter(|c| c.name.is_empty()) {
                if let Some(info) = chat_info(&t.token, &c.id) {
                    c.name = info.name;
                    c.username = info.username;
                }
            }
            let _ = st.tg.set(Some(t));
        }
        let mut offset: i64 = 0;
        loop {
            let Some(tg) = st.tg.get() else {
                std::thread::sleep(Duration::from_secs(30));
                continue;
            };
            let url = format!("https://api.telegram.org/bot{}/getUpdates?timeout=25&offset={offset}", tg.token);
            let upd: serde_json::Value =
                match agent().get(&url).call().map_err(api_err).and_then(|r| r.into_json().map_err(|e| e.to_string())) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[telegram] poll: {e}");
                        std::thread::sleep(Duration::from_secs(30));
                        continue;
                    }
                };
            for u in upd["result"].as_array().cloned().unwrap_or_default() {
                offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                let msg = &u["message"];
                let Some(chat_id) = msg["chat"]["id"].as_i64().map(|c| c.to_string()) else { continue };
                let text = msg["text"].as_str().unwrap_or("").trim();
                let cmd = text.split(['@', ' ']).next().unwrap_or("");
                let who = chat_from(&msg["chat"], &chat_id);
                let allowed = tg.chats.iter().any(|c| c.id == chat_id);
                let reply = if allowed {
                    st.tg.refresh_chat(&who);
                    match cmd {
                        "/status" | "/s" => status_text(&st),
                        "/help" | "/start" => HELP.to_string(),
                        "/id" => format!("chat id: {chat_id}"),
                        _ => continue,
                    }
                } else {
                    match cmd {
                        "/start" | "/id" => {
                            eprintln!("[telegram] access request from {} ({chat_id})", who.name);
                            st.tg.add_pending(who);
                            format!("Запрос на доступ отправлен владельцу дашборда. Ваш chat id: {chat_id}")
                        }
                        _ => continue,
                    }
                };
                if let Err(e) = send_one(&tg.token, &chat_id, &reply) {
                    eprintln!("[telegram] reply: {e}");
                }
            }
        }
    });
}
