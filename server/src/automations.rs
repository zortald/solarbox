//! Automations: "when <condition> (and …) holds for N s → do actions; when it
//! clears → do other actions". Evaluated server-side on every ingested packet
//! and on a 1 s tick (hold times, staleness), so they run with no browser open.
//!
//! Rules live in `automations.json` on the data volume (hot-reloaded on
//! change, replaceable via the admin API). Board actions go through the same
//! allow-rules as manual control, so an automation can't do more than an
//! admin could by hand.

use crate::{boards, AppState};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cond {
    pub device: String,
    pub metric: String,
    /// > < >= <= == != | "between" (value..value2, wraps past midnight for the
    /// clock) | "changed" (momentary) | "stale" (no data for `value` s)
    pub op: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value2: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    /// "board" (control a board entity) | "telegram" (message, {device.metric}
    /// placeholders) | "log" (journal entry only)
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    /// "domain/entity name", e.g. "switch/inverter output_source_priority_battery"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Pause before this action, seconds.
    #[serde(default)]
    pub delay_s: u64,
}

fn yes() -> bool { true }
fn sixty() -> u64 { 60 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub when: Cond,
    #[serde(default)]
    pub and: Vec<Cond>,
    /// Condition must hold this long before firing.
    #[serde(default)]
    pub for_s: u64,
    /// Minimum time between two firings.
    #[serde(default = "sixty")]
    pub cooldown_s: u64,
    #[serde(default)]
    pub then: Vec<Action>,
    /// Run when the condition stops holding after having fired.
    #[serde(default)]
    pub on_clear: Vec<Action>,
}

#[derive(Default, Clone, Serialize)]
pub struct RuleState {
    #[serde(skip)]
    since: Option<Instant>,
    pub active: bool,
    pub fired: bool,
    pub last_fired: Option<i64>,
    #[serde(skip)]
    last_fire_at: Option<Instant>,
}

#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub ts: i64,
    pub rule: String,
    pub msg: String,
}

struct Inner {
    rules: Vec<Rule>,
    state: HashMap<String, RuleState>,
    values: HashMap<(String, String), (f64, i64)>,
    mtime: Option<SystemTime>,
    load_error: Option<String>,
    /// Fired/last_fired changed since the last save to the database.
    dirty: bool,
}

pub struct Engine {
    path: PathBuf,
    inner: Mutex<Inner>,
    log: Mutex<VecDeque<LogEntry>>,
    /// When attached, the journal is kept in SQLite (survives restarts).
    db: Mutex<Option<std::sync::Arc<Mutex<rusqlite::Connection>>>>,
}

const LOG_KEEP: i64 = 500;

fn now_ts() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn validate(rules: &[Rule]) -> Result<(), String> {
    let ops = [">", "<", ">=", "<=", "==", "!=", "between", "changed", "stale"];
    let mut ids = std::collections::HashSet::new();
    for r in rules {
        if r.id.is_empty() || !ids.insert(r.id.clone()) {
            return Err(format!("rule id «{}» is empty or duplicated", r.id));
        }
        for c in std::iter::once(&r.when).chain(r.and.iter()) {
            if !ops.contains(&c.op.as_str()) {
                return Err(format!("{}: unknown operator «{}»", r.name, c.op));
            }
            if c.op == "between" && c.value2.is_none() {
                return Err(format!("{}: «between» needs value2", r.name));
            }
        }
        for a in r.then.iter().chain(r.on_clear.iter()) {
            match a.kind.as_str() {
                "log" => {}
                "telegram" => {
                    if a.message.as_deref().unwrap_or("").trim().is_empty() {
                        return Err(format!("{}: telegram action needs a message", r.name));
                    }
                }
                "board" => {
                    if a.board.is_none() || a.path.is_none() || a.action.is_none() {
                        return Err(format!("{}: board action needs board, path and action", r.name));
                    }
                }
                k => return Err(format!("{}: unknown action kind «{k}»", r.name)),
            }
        }
    }
    Ok(())
}

impl Engine {
    pub fn open(path: PathBuf) -> Self {
        if !path.exists() {
            let _ = std::fs::write(&path, "[]\n");
        }
        let e = Engine {
            path,
            inner: Mutex::new(Inner { rules: vec![], state: HashMap::new(), values: HashMap::new(), mtime: None, load_error: None, dirty: false }),
            log: Mutex::new(VecDeque::new()),
            db: Mutex::new(None),
        };
        e.reload_if_changed();
        e
    }

    pub fn reload_if_changed(&self) {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        let mut g = self.inner.lock().unwrap();
        if mtime.is_none() || mtime == g.mtime {
            return;
        }
        g.mtime = mtime;
        let parsed = std::fs::read_to_string(&self.path).map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Vec<Rule>>(&s).map_err(|e| e.to_string()))
            .and_then(|r| validate(&r).map(|_| r));
        match parsed {
            Ok(rules) => {
                eprintln!("[auto] loaded {} rule(s)", rules.len());
                // Keep state of rules that still exist (by id).
                let keep: HashMap<_, _> = rules.iter().filter_map(|r| g.state.get(&r.id).map(|s| (r.id.clone(), s.clone()))).collect();
                g.state = keep;
                g.rules = rules;
                g.load_error = None;
            }
            Err(e) => {
                eprintln!("[auto] {}: {e} — keeping previous rules", self.path.display());
                g.load_error = Some(e);
            }
        }
    }

    pub fn save(&self, text: &str) -> Result<usize, String> {
        let rules: Vec<Rule> = serde_json::from_str(text).map_err(|e| format!("invalid automations: {e}"))?;
        validate(&rules)?;
        let pretty = serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, pretty).and_then(|_| std::fs::rename(&tmp, &self.path)).map_err(|e| e.to_string())?;
        self.reload_if_changed();
        Ok(rules.len())
    }

    /// Keep the journal in the database from now on.
    pub fn attach_db(&self, db: std::sync::Arc<Mutex<rusqlite::Connection>>) {
        if let Err(e) = db.lock().unwrap().execute_batch(
            "CREATE TABLE IF NOT EXISTS auto_log(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, rule TEXT NOT NULL, msg TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS auto_state(id TEXT PRIMARY KEY, fired INTEGER NOT NULL, last_fired INTEGER);",
        ) {
            eprintln!("[auto] journal table: {e}");
            return;
        }
        // Restore which rules already fired, so a restart during e.g. an
        // outage doesn't announce the same outage again.
        let mut saved: Vec<(String, bool, Option<i64>)> = {
            let c = db.lock().unwrap();
            c.prepare("SELECT id, fired, last_fired FROM auto_state")
                .and_then(|mut st| st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
                .unwrap_or_default()
        };
        if saved.is_empty() {
            // First run with this table: take it from the journal — a rule whose
            // latest entry is «сработало» (not cleared since) has fired.
            let rules = self.inner.lock().unwrap().rules.clone();
            let c = db.lock().unwrap();
            for r in rules.iter().filter(|r| r.when.op != "changed") {
                let last: Option<(i64, String)> = c
                    .query_row(
                        "SELECT ts, msg FROM auto_log WHERE rule = ?1 AND (msg = 'сработало' OR msg LIKE 'условие снято%')
                         ORDER BY id DESC LIMIT 1",
                        [&r.name],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok();
                if let Some((ts, msg)) = last {
                    saved.push((r.id.clone(), msg == "сработало", Some(ts)));
                }
            }
        }
        {
            let mut g = self.inner.lock().unwrap();
            for (id, fired, last) in saved {
                let st = g.state.entry(id).or_default();
                st.fired = fired;
                st.active = fired;
                st.last_fired = last;
            }
        }
        *self.db.lock().unwrap() = Some(db);
    }

    /// Save fired state if it changed. Called from the 1 s tick loop, never
    /// while the database lock is held by ingest.
    fn persist_state(&self) {
        let rows: Vec<(String, bool, Option<i64>)> = {
            let mut g = self.inner.lock().unwrap();
            if !g.dirty {
                return;
            }
            g.dirty = false;
            g.state.iter().map(|(id, s)| (id.clone(), s.fired, s.last_fired)).collect()
        };
        if let Some(db) = self.db.lock().unwrap().as_ref() {
            let c = db.lock().unwrap();
            let r = c.execute("DELETE FROM auto_state", []).and_then(|_| {
                let mut ins = c.prepare_cached("INSERT INTO auto_state(id, fired, last_fired) VALUES(?1, ?2, ?3)")?;
                for (id, fired, last) in &rows {
                    ins.execute(rusqlite::params![id, fired, last])?;
                }
                Ok(())
            });
            if let Err(e) = r {
                eprintln!("[auto] state: {e}");
            }
        }
    }

    pub fn log(&self, rule: &str, msg: impl Into<String>) {
        let msg = msg.into();
        eprintln!("[auto] {rule}: {msg}");
        let entry = LogEntry { ts: now_ts(), rule: rule.to_string(), msg };
        if let Some(db) = self.db.lock().unwrap().as_ref() {
            let c = db.lock().unwrap();
            let r = c
                .execute("INSERT INTO auto_log(ts, rule, msg) VALUES(?1, ?2, ?3)", rusqlite::params![entry.ts, entry.rule, entry.msg])
                .and_then(|_| c.execute("DELETE FROM auto_log WHERE id <= (SELECT MAX(id) FROM auto_log) - ?1", [LOG_KEEP]));
            if let Err(e) = r {
                eprintln!("[auto] journal: {e}");
            }
            return;
        }
        let mut l = self.log.lock().unwrap();
        l.push_front(entry);
        l.truncate(200);
    }

    fn journal(&self) -> Vec<LogEntry> {
        if let Some(db) = self.db.lock().unwrap().as_ref() {
            let c = db.lock().unwrap();
            let rows = c.prepare("SELECT ts, rule, msg FROM auto_log ORDER BY id DESC LIMIT 200").and_then(|mut st| {
                st.query_map([], |r| Ok(LogEntry { ts: r.get(0)?, rule: r.get(1)?, msg: r.get(2)? }))?.collect()
            });
            return rows.unwrap_or_default();
        }
        self.log.lock().unwrap().iter().cloned().collect()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let log = self.journal();
        let g = self.inner.lock().unwrap();
        serde_json::json!({
            "rules": g.rules,
            "state": g.state,
            "load_error": g.load_error,
            "log": log,
        })
    }

    /// Current value of a metric (as last pushed).
    pub fn value(&self, device: &str, metric: &str) -> Option<f64> {
        self.inner.lock().unwrap().values.get(&(device.to_string(), metric.to_string())).map(|(v, _)| *v)
    }

    pub fn rules(&self) -> Vec<Rule> {
        self.inner.lock().unwrap().rules.clone()
    }
    pub fn rule(&self, id: &str) -> Option<Rule> {
        self.inner.lock().unwrap().rules.iter().find(|r| r.id == id).cloned()
    }

    /// Feed one ingested packet; returns rules whose actions must run now.
    pub fn on_packet(&self, device: &str, ts: i64, metrics: &HashMap<String, f64>) -> Vec<(Rule, bool)> {
        let mut g = self.inner.lock().unwrap();
        let mut changed = Vec::new();
        for (m, v) in metrics {
            let key = (device.to_string(), m.clone());
            if let Some((old, _)) = g.values.get(&key) {
                if (old - v).abs() > f64::EPSILON {
                    changed.push(m.clone());
                }
            }
            g.values.insert(key, (*v, ts));
        }
        evaluate(&mut g, Some((device, &changed)))
    }

    /// Set a derived value (clock, today's energy…) usable in conditions.
    pub fn set_value(&self, device: &str, metric: &str, v: f64, ts: i64) {
        self.inner.lock().unwrap().values.insert((device.to_string(), metric.to_string()), (v, ts));
    }

    /// Periodic evaluation (clock, hold times, staleness).
    pub fn tick(&self, tz: &crate::tz::Tz) -> Vec<(Rule, bool)> {
        self.reload_if_changed();
        let now = now_ts();
        let mut g = self.inner.lock().unwrap();
        let minute = tz.minute_of_day(now);
        for (m, v) in [("minute", minute), ("hour", minute / 60), ("weekday", tz.weekday(now))] {
            g.values.insert(("clock".to_string(), m.to_string()), (v as f64, now));
        }
        let out = evaluate(&mut g, None);
        drop(g);
        self.persist_state();
        out
    }
}

fn holds(values: &HashMap<(String, String), (f64, i64)>, c: &Cond, now: i64) -> bool {
    let cur = values.get(&(c.device.clone(), c.metric.clone()));
    match c.op.as_str() {
        "stale" => {
            // "*" = newest metric of the device
            let newest = if c.metric == "*" {
                values.iter().filter(|((d, _), _)| d == &c.device).map(|(_, (_, t))| *t).max()
            } else {
                cur.map(|(_, t)| *t)
            };
            newest.map(|t| (now - t) as f64 > c.value).unwrap_or(true)
        }
        "changed" => false, // momentary, handled separately
        op => match cur {
            None => false,
            Some((v, _)) => match op {
                ">" => *v > c.value,
                "<" => *v < c.value,
                ">=" => *v >= c.value,
                "<=" => *v <= c.value,
                "==" => (*v - c.value).abs() < 1e-9,
                "!=" => (*v - c.value).abs() >= 1e-9,
                // inclusive; a > b wraps (e.g. 23:00..07:00 on the clock)
                "between" => {
                    let b = c.value2.unwrap_or(c.value);
                    if c.value <= b { *v >= c.value && *v <= b } else { *v >= c.value || *v <= b }
                }
                _ => false,
            },
        },
    }
}

/// Is there a value to judge `c` by? «stale» is about missing data itself.
fn known(values: &HashMap<(String, String), (f64, i64)>, c: &Cond) -> bool {
    c.op == "stale" || c.op == "changed" || values.contains_key(&(c.device.clone(), c.metric.clone()))
}

/// Returns (rule, is_clear) pairs to execute.
fn evaluate(g: &mut Inner, packet: Option<(&str, &Vec<String>)>) -> Vec<(Rule, bool)> {
    let now = Instant::now();
    let nts = now_ts();
    let mut out = Vec::new();
    let mut dirty = false;
    let rules = g.rules.clone();
    for r in rules.iter().filter(|r| r.enabled) {
        // No value yet (e.g. right after a restart, before the first packet):
        // neither fire nor clear — keep the state as it was.
        if !known(&g.values, &r.when) || !r.and.iter().all(|c| known(&g.values, c)) {
            continue;
        }
        let others_ok = r.and.iter().all(|c| holds(&g.values, c, nts));
        let st = g.state.entry(r.id.clone()).or_default();
        let cooled = st.last_fire_at.map(|t| now.duration_since(t) >= Duration::from_secs(r.cooldown_s)).unwrap_or(true);

        if r.when.op == "changed" {
            let hit = matches!(packet, Some((d, ch)) if d == r.when.device && ch.contains(&r.when.metric));
            if hit && others_ok && cooled {
                st.last_fire_at = Some(now);
                st.last_fired = Some(nts);
                out.push((r.clone(), false));
                dirty = true;
            }
            continue;
        }

        let cond = holds(&g.values, &r.when, nts) && others_ok;
        if cond {
            let since = *st.since.get_or_insert(now);
            st.active = true;
            if !st.fired && cooled && now.duration_since(since) >= Duration::from_secs(r.for_s) {
                st.fired = true;
                st.last_fire_at = Some(now);
                st.last_fired = Some(nts);
                out.push((r.clone(), false));
                dirty = true;
            }
        } else {
            st.since = None;
            st.active = false;
            if st.fired {
                st.fired = false;
                dirty = true;
                if !r.on_clear.is_empty() {
                    out.push((r.clone(), true));
                }
            }
        }
    }
    g.dirty |= dirty;
    out
}

/// Run a rule's actions (or its on_clear actions) in order, with delays.
pub async fn execute(st: AppState, rule: Rule, clear: bool) {
    let actions = if clear { &rule.on_clear } else { &rule.then };
    st.autom.log(&rule.name, if clear { "условие снято → обратные действия" } else { "сработало" });
    for a in actions {
        if a.delay_s > 0 {
            tokio::time::sleep(Duration::from_secs(a.delay_s)).await;
        }
        match a.kind.as_str() {
            "log" => st.autom.log(&rule.name, crate::telegram::render(&st, a.message.as_deref().unwrap_or(""))),
            "telegram" => {
                let text = crate::telegram::render(&st, a.message.as_deref().unwrap_or(""));
                match crate::telegram::send(&st, text.clone()).await {
                    Ok(_) => st.autom.log(&rule.name, format!("✔ Telegram: {text}")),
                    Err(e) => st.autom.log(&rule.name, format!("✘ {e}")),
                }
            }
            "board" => {
                let (Some(board), Some(path), Some(action)) = (&a.board, &a.path, &a.action) else { continue };
                let Some((domain, ent)) = path.split_once('/') else {
                    st.autom.log(&rule.name, format!("неверный путь {path}"));
                    continue;
                };
                if !st.config.get().allows(board, domain, ent, action, a.value.as_deref()) {
                    st.autom.log(&rule.name, format!("запрещено правилами: {action} {path}"));
                    continue;
                }
                let Some(b) = boards::find(&st, board) else {
                    st.autom.log(&rule.name, format!("нет платы {board}"));
                    continue;
                };
                let mut url = format!("/{domain}/{}/{action}", boards::pct(ent));
                if let Some(v) = &a.value {
                    url.push_str(&format!("?value={}", boards::pct(v)));
                }
                let what = format!("{board}: {action} {ent}{}", a.value.as_deref().map(|v| format!(" = {v}")).unwrap_or_default());
                match boards::http(&b, "POST", &url).await {
                    Ok((c, _)) if c < 300 => {
                        st.autom.log(&rule.name, format!("✔ {what}"));
                        let text = crate::changes::describe_action(&st, board, domain, ent, action, a.value.as_deref());
                        crate::changes::notify(&st, text, crate::changes::By::Automation(&rule.name));
                    }
                    Ok((c, body)) => st.autom.log(&rule.name, format!("✘ {what}: HTTP {c} {}", body.trim())),
                    Err(e) => st.autom.log(&rule.name, format!("✘ {what}: {e}")),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(op: &str, value: f64, for_s: u64) -> Rule {
        Rule {
            id: "r".into(), name: "r".into(), enabled: true,
            when: Cond { device: "inverter".into(), metric: "grid_v".into(), op: op.into(), value, value2: None },
            and: vec![], for_s, cooldown_s: 0,
            then: vec![Action { kind: "log".into(), board: None, path: None, action: None, value: None, message: Some("x".into()), delay_s: 0 }],
            on_clear: vec![Action { kind: "log".into(), board: None, path: None, action: None, value: None, message: Some("y".into()), delay_s: 0 }],
        }
    }
    fn inner(rules: Vec<Rule>) -> Inner {
        Inner { rules, state: HashMap::new(), values: HashMap::new(), mtime: None, load_error: None, dirty: false }
    }
    fn feed(g: &mut Inner, v: f64) -> Vec<(Rule, bool)> {
        g.values.insert(("inverter".into(), "grid_v".into()), (v, now_ts()));
        evaluate(g, Some(("inverter", &vec!["grid_v".into()])))
    }

    #[test]
    fn edge_and_clear() {
        let mut g = inner(vec![rule("<", 150.0, 0)]);
        assert!(feed(&mut g, 230.0).is_empty());
        let f = feed(&mut g, 0.0);
        assert_eq!(f.len(), 1);
        assert!(!f[0].1);
        assert!(feed(&mut g, 0.0).is_empty(), "fires once while held");
        let c = feed(&mut g, 231.0);
        assert_eq!(c.len(), 1);
        assert!(c[0].1, "on_clear when condition stops");
    }

    #[test]
    fn restored_fired_state_does_not_refire() {
        // as after a restart during an outage: state says it already fired
        let mut g = inner(vec![rule("<", 150.0, 0)]);
        g.state.insert("r".into(), RuleState { fired: true, active: true, ..Default::default() });
        assert!(feed(&mut g, 0.0).is_empty(), "same outage must not be announced again");
        assert!(!g.dirty);
        let out = feed(&mut g, 230.0);
        assert_eq!(out.len(), 1, "grid back → on_clear still runs");
        assert!(out[0].1 && g.dirty);
    }

    #[test]
    fn no_data_yet_keeps_state() {
        let mut g = inner(vec![rule("<", 150.0, 0)]);
        g.state.insert("r".into(), RuleState { fired: true, active: true, ..Default::default() });
        assert!(evaluate(&mut g, None).is_empty(), "no grid_v yet → no false «grid back»");
        assert!(g.state["r"].fired);
    }

    #[test]
    fn hold_time() {
        let mut g = inner(vec![rule("<", 150.0, 3600)]);
        assert!(feed(&mut g, 0.0).is_empty(), "must hold for an hour first");
    }

    #[test]
    fn clock_between_wraps() {
        let mut r = rule("between", 23.0 * 60.0, 0);
        r.when = Cond { device: "clock".into(), metric: "minute".into(), op: "between".into(), value: 23.0 * 60.0, value2: Some(7.0 * 60.0) };
        let mut g = inner(vec![r]);
        let at = |g: &mut Inner, min: i64| { g.values.insert(("clock".into(), "minute".into()), (min as f64, now_ts())); evaluate(g, None) };
        assert!(at(&mut g, 22 * 60).is_empty());
        assert_eq!(at(&mut g, 23 * 60 + 5).len(), 1, "23:05 is inside 23:00–07:00");
        assert!(at(&mut g, 3 * 60).is_empty(), "still inside, fires once");
        let c = at(&mut g, 7 * 60 + 1);
        assert!(c.len() == 1 && c[0].1, "07:01 → on_clear");
    }

    #[test]
    fn validation() {
        assert!(validate(&[rule("<", 1.0, 0)]).is_ok());
        assert!(validate(&[rule("~", 1.0, 0)]).is_err());
        assert!(validate(&[rule("<", 1.0, 0), rule("<", 1.0, 0)]).is_err(), "duplicate id");
    }
}
