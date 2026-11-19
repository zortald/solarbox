//! Setup wizard backend. While the owner steps through the wizard nothing is
//! applied: the answers are a draft kept in `wizard.json` (an interrupted
//! wizard resumes). The review screen asks for the plan — every change with
//! its old and new value — and «Применить» carries it out item by item; a
//! failed item is reported and can be retried, the rest stays applied.
//!
//! Draft fields read here (the rest belongs to the UI):
//!   site.name · money {day, night, nday, from, to} · tg.token ·
//!   adopt [{mac, name, title, chip, ip, slots: [{slot, bms_mac, proto, device}]}]

use crate::{boards, changes, config::BoardCfg, AppState};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Mutex};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Saved {
    /// "new" (never finished), "done", "skipped".
    #[serde(default = "d_state")]
    pub state: String,
    #[serde(default)]
    pub draft: Value,
}
fn d_state() -> String {
    "new".into()
}

pub struct Store {
    path: PathBuf,
    dir: PathBuf,
    m: Mutex<Saved>,
}

impl Store {
    pub fn open(dir: &std::path::Path) -> Store {
        let path = dir.join("wizard.json");
        let s = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Saved { state: d_state(), draft: json!({}) });
        Store { path, dir: dir.to_path_buf(), m: Mutex::new(s) }
    }
    pub fn get(&self) -> Saved {
        self.m.lock().unwrap().clone()
    }
    fn put(&self, f: impl FnOnce(&mut Saved)) -> Result<(), String> {
        let mut g = self.m.lock().unwrap();
        f(&mut g);
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&*g).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

/// One change on the review screen.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub id: String,
    pub group: &'static str,
    pub title: String,
    pub old: String,
    pub new: String,
    #[serde(skip)]
    act: Act,
}

#[derive(Clone, Debug)]
enum Act {
    SiteName(String),
    Tariff(crate::money::Tariff),
    Telegram(String),
    Adopt(BoardCfg),
    /// POST to the board's web API: (ip, path).
    Board(String, String),
}

fn st_(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// ESPHome entity names of slot `n` on the JK BLE board (slot 1 plain, N > 1 " #N").
fn ent(base: &str, n: u64) -> String {
    if n <= 1 { base.to_string() } else { format!("{base} #{n}") }
}

/// The plan: what applying the draft would change, against the current state.
pub fn plan(st: &AppState, draft: &Value) -> Vec<Item> {
    let cfg = st.config.get();
    let disc = st.discovery.boards(&cfg, crate::now());
    let mut out = Vec::new();
    let mut push = |id: String, group, title: String, old: String, new: String, act| out.push(Item { id, group, title, old, new, act });

    let name = draft["site"]["name"].as_str().unwrap_or("").trim().to_string();
    if !name.is_empty() && name != cfg.site_name {
        push("site".into(), "Solarbox", "Название объекта".into(), cfg.site_name.clone(), name.clone(), Act::SiteName(name));
    }

    let m = &draft["money"];
    if let Some(day) = m["day"].as_f64().filter(|p| *p > 0.0) {
        let night = m["night"].as_bool().unwrap_or(false);
        let t = crate::money::Tariff {
            from: cfg.timezone.day(crate::now()),
            price: day,
            night_price: if night { m["nday"].as_f64().filter(|p| *p > 0.0) } else { None },
            night_from: m["from"].as_u64().unwrap_or(23).min(23) as u8,
            night_to: m["to"].as_u64().unwrap_or(7).min(23) as u8,
        };
        let cur = cfg.money.tariffs.iter().filter(|x| x.from <= t.from).max_by(|a, b| a.from.cmp(&b.from));
        let show = |t: &crate::money::Tariff| match t.night_price {
            Some(n) => format!("{} грн, ночью {} грн ({}–{})", t.price, n, t.night_from, t.night_to),
            None => format!("{} грн за кВт·ч", t.price),
        };
        let same = cur.is_some_and(|c| c.price == t.price && c.night_price == t.night_price && (t.night_price.is_none() || (c.night_from, c.night_to) == (t.night_from, t.night_to)));
        if !same {
            push("tariff".into(), "Solarbox", format!("Тариф с {}", t.from), cur.map(show).unwrap_or_default(), show(&t), Act::Tariff(t));
        }
    }

    let token = draft["tg"]["token"].as_str().unwrap_or("").trim().to_string();
    if !token.is_empty() && st.tg.get().map(|t| t.token) != Some(token.clone()) {
        let masked = format!("{}…{}", &token[..token.find(':').unwrap_or(0).min(token.len())], &token[token.len().saturating_sub(4)..]);
        let old = st.tg.get().and_then(|t| t.bot).map(|b| format!("@{b}")).unwrap_or_default();
        push("telegram".into(), "Solarbox", "Telegram-бот".into(), old, masked, Act::Telegram(token));
    }

    for a in draft["adopt"].as_array().into_iter().flatten() {
        let Some(mac) = a["mac"].as_str().map(|m| m.to_ascii_uppercase()) else { continue };
        let seen = disc.iter().find(|d| d["mac"] == mac.as_str());
        let ip = seen.and_then(|d| d["ip"].as_str()).or(a["ip"].as_str()).unwrap_or("").to_string();
        let name = a["name"].as_str().unwrap_or("").to_string();
        let title = a["title"].as_str().unwrap_or(&name).to_string();
        if cfg.board_by_mac(&mac).is_none() && !name.is_empty() && !ip.is_empty() {
            let mut ui = serde_json::Map::new();
            ui.insert("title".into(), json!(title));
            if let Some(c) = a["chip"].as_str() {
                ui.insert("chip".into(), json!(c));
            }
            ui.insert("restart".into(), json!("button/Перезагрузка RST"));
            let b = BoardCfg { name: name.clone(), host: ip.clone(), web_port: 80, ota_port: 3232, mac: Some(mac.clone()), ui };
            push(format!("adopt:{mac}"), "Платы", format!("Подключить плату «{title}»"), String::new(), format!("{mac}, {ip}"), Act::Adopt(b));
        }
        // Settings written into the board itself (its own flash), slot by slot.
        for s in a["slots"].as_array().into_iter().flatten() {
            let n = s["slot"].as_u64().unwrap_or(1);
            let now = seen.and_then(|d| d["slots"].as_array()).and_then(|v| v.iter().find(|x| x["slot"].as_u64() == Some(n)));
            let cur = |k: &str| now.map(|x| st_(&x[k])).unwrap_or_default();
            let want = |k: &str| st_(&s[k]);
            let mut set = |key: &str, ent_name: &str, param: &str, label: &str| {
                let (old, new) = (cur(key), want(key));
                if !new.is_empty() && !old.eq_ignore_ascii_case(&new) {
                    let path = format!("/{}/{}/set?{param}={}", if param == "option" { "select" } else { "text" }, boards::pct(&ent(ent_name, n)), boards::pct(&new));
                    push(format!("board:{mac}:{n}:{key}"), "Настройки плат", format!("«{title}», BMS {n}: {label}"), old, new, Act::Board(ip.clone(), path));
                }
            };
            set("bms_mac", "BMS MAC", "value", "MAC");
            set("proto", "Протокол BMS", "option", "протокол");
            set("device", "Имя в Solarbox", "value", "имя");
        }
    }
    out
}

async fn run(st: &AppState, it: &Item) -> Result<(), String> {
    match &it.act {
        Act::SiteName(n) => st.config.update(|c| c.site_name = n.clone()),
        Act::Tariff(t) => st.config.update(|c| {
            c.money.tariffs.retain(|x| x.from != t.from);
            c.money.tariffs.push(t.clone());
            c.money.tariffs.sort_by(|a, b| a.from.cmp(&b.from));
        }),
        Act::Adopt(b) => st.config.update(|c| {
            c.boards.retain(|x| x.name != b.name && !x.mac.as_deref().is_some_and(|m| Some(m) == b.mac.as_deref()));
            c.boards.push(b.clone());
        }),
        Act::Telegram(token) => {
            let tk = token.clone();
            let bot = tokio::task::spawn_blocking(move || crate::telegram::setup_bot(&tk)).await.map_err(|e| e.to_string())?.map_err(|e| format!("токен не подошёл: {e}"))?;
            let prev = st.tg.get();
            let mut tg = crate::telegram::Tg::default();
            tg.token = token.clone();
            tg.chats = prev.as_ref().map(|t| t.chats.clone()).unwrap_or_default();
            tg.notify_changes = prev.as_ref().map(|t| t.notify_changes).unwrap_or(true);
            tg.notify_access = prev.as_ref().map(|t| t.notify_access).unwrap_or(true);
            tg.bot = Some(bot);
            st.tg.set(Some(tg))
        }
        Act::Board(ip, path) => {
            let b = boards::Board { name: ip.clone(), host: ip.clone(), web_port: 80, ota_port: 3232 };
            match boards::http(&b, "POST", path).await? {
                (c, _) if c < 300 => Ok(()),
                (c, body) => Err(format!("плата ответила HTTP {c}: {}", body.trim())),
            }
        }
    }
}

// ---- handlers ---------------------------------------------------------------

fn bad(code: StatusCode, e: impl Into<String>) -> Response {
    (code, Json(json!({ "ok": false, "error": e.into() }))).into_response()
}

/// GET /api/wizard — state, draft, and whether the wizard should open by itself.
pub async fn get(State(st): State<AppState>) -> Json<Value> {
    let s = st.wizard.get();
    let auto = s.state == "new" && st.config.get().boards.is_empty();
    Json(json!({ "state": s.state, "draft": s.draft, "auto": auto }))
}

#[derive(Deserialize)]
pub struct DraftBody {
    draft: Value,
}

/// PUT /api/wizard — keep the draft (nothing is applied).
pub async fn put(State(st): State<AppState>, Json(b): Json<DraftBody>) -> Response {
    match st.wizard.put(|s| s.draft = b.draft) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// POST /api/wizard/skip — don't open by itself any more.
pub async fn skip(State(st): State<AppState>) -> Response {
    match st.wizard.put(|s| if s.state == "new" { s.state = "skipped".into() }) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// GET /api/wizard/emergency — the emergency password for the last screen
/// (the person is logged in; it also sits in /data/admin-password).
pub async fn emergency(State(st): State<AppState>) -> Response {
    match std::fs::read_to_string(st.wizard.dir.join("admin-password")) {
        Ok(p) => Json(json!({ "password": p.trim() })).into_response(),
        Err(_) => bad(StatusCode::NOT_FOUND, "задан через переменную окружения"),
    }
}

/// GET /api/wizard/plan — the review screen.
pub async fn plan_api(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "items": plan(&st, &st.wizard.get().draft) }))
}

#[derive(Deserialize)]
pub struct ApplyBody {
    /// Only these item ids (a retry); none = everything in the plan.
    #[serde(default)]
    only: Option<Vec<String>>,
}

/// POST /api/wizard/apply — carry out the plan item by item.
pub async fn apply(State(st): State<AppState>, Json(b): Json<ApplyBody>) -> Response {
    let items = plan(&st, &st.wizard.get().draft);
    let mut results = Vec::new();
    for it in items.iter().filter(|it| b.only.as_ref().map_or(true, |o| o.contains(&it.id))) {
        let r = run(&st, it).await;
        eprintln!("[wizard] {}: {}", it.title, r.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.clone()));
        if r.is_ok() {
            changes::notify(&st, format!("Мастер настройки: {} — {}", it.title, it.new), changes::By::Dashboard);
        }
        results.push(json!({ "id": it.id, "ok": r.is_ok(), "error": r.err() }));
    }
    // What a board reports back arrives with its next packet (≈5 s); what
    // was just written successfully counts as done meanwhile.
    let ok: Vec<&str> = results.iter().filter(|r| r["ok"] == true).filter_map(|r| r["id"].as_str()).collect();
    let left = plan(&st, &st.wizard.get().draft).iter().filter(|i| !ok.contains(&i.id.as_str())).count();
    if left == 0 {
        let _ = st.wizard.put(|s| s.state = "done".into());
    }
    Json(json!({ "results": results, "left": left })).into_response()
}

#[cfg(test)]
mod tests {
    use super::ent;
    #[test]
    fn entity_names() {
        assert_eq!(ent("BMS MAC", 1), "BMS MAC");
        assert_eq!(ent("BMS MAC", 2), "BMS MAC #2");
    }
}
