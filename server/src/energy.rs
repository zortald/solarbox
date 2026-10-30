//! Daily energy: power is integrated on every pushed packet (before any
//! history thinning) into per-local-day totals — PV, load, battery charge and
//! discharge, and the estimated grid draw (load + battery − PV, inverter has no
//! grid meter). Gaps longer than `MAX_GAP_S` aren't integrated.

use crate::{tz::Tz, AppState};
use axum::{extract::{Query, State}, http::StatusCode, response::{IntoResponse, Response}, Json};
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::{collections::HashMap, sync::Mutex};

const MAX_GAP_S: i64 = 120;
/// Below this the "grid" residual is the inverter's own losses, not grid draw.
const GRID_MIN_W: f64 = 40.0;
pub const KEYS: [&str; 5] = ["pv", "load", "chg", "dis", "grid"];
/// Daily extremes kept exactly (history thinning would flatten short peaks):
/// key → (device, metric, is_max).
const EXT: [(&str, &str, &str, bool); 4] = [
    ("pv_max", "inverter", "pv_w", true),
    ("load_max", "inverter", "ac_out_w", true),
    ("soc_min", "bms", "soc", false),
    ("soc_max", "bms", "soc", true),
];

#[derive(Default)]
struct S {
    latest: HashMap<&'static str, f64>,        // pv_w, ac_out_w, bms_w, grid_v
    last: HashMap<&'static str, (i64, f64)>,   // integrated stream → (ts, watts)
    ext: HashMap<(String, &'static str), f64>, // (day, key) → extreme so far
}

pub struct Energy {
    s: Mutex<S>,
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS energy_daily(
             day TEXT NOT NULL,
             key TEXT NOT NULL,
             wh  REAL NOT NULL,
             PRIMARY KEY(day, key)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS energy_ext(
             day TEXT NOT NULL,
             key TEXT NOT NULL,
             v   REAL NOT NULL,
             PRIMARY KEY(day, key)
         ) WITHOUT ROWID;",
    )
}

impl Energy {
    pub fn new() -> Self {
        Energy { s: Mutex::new(S::default()) }
    }

    /// Feed one packet; returns watt-hours added per (day, key).
    pub fn feed(&self, device: &str, ts: i64, metrics: &HashMap<String, f64>, tz: &Tz) -> Vec<(String, &'static str, f64)> {
        let mut s = self.s.lock().unwrap();
        let mut streams: Vec<(&'static str, f64)> = Vec::new();
        let take = |s: &mut S, name: &'static str| {
            if let Some(v) = metrics.get(name).filter(|v| v.is_finite()) {
                s.latest.insert(name, *v);
                true
            } else {
                false
            }
        };
        match device {
            "inverter" => {
                let pv = take(&mut s, "pv_w");
                let load = take(&mut s, "ac_out_w");
                take(&mut s, "grid_v");
                if pv { streams.push(("pv", s.latest["pv_w"])); }
                if load { streams.push(("load", s.latest["ac_out_w"])); }
                if load {
                    let g = s.latest.get("ac_out_w").copied().unwrap_or(0.0)
                        + s.latest.get("bms_w").copied().unwrap_or(0.0)
                        - s.latest.get("pv_w").copied().unwrap_or(0.0);
                    let up = s.latest.get("grid_v").is_some_and(|v| *v > 150.0);
                    streams.push(("grid", if up && g > GRID_MIN_W { g } else { 0.0 }));
                }
            }
            "bms" => {
                if take(&mut s, "bms_w") { streams.push(("batt", s.latest["bms_w"])); }
            }
            _ => {}
        }
        let mut out = Vec::new();
        for (stream, p) in streams {
            if let Some((t0, p0)) = s.last.insert(stream, (ts, p)) {
                let dt = ts - t0;
                if dt <= 0 || dt > MAX_GAP_S {
                    continue;
                }
                let wh = p0 * dt as f64 / 3600.0;
                let day = tz.day(t0);
                match stream {
                    "batt" if wh > 0.0 => out.push((day, "chg", wh)),
                    "batt" => out.push((day, "dis", -wh)),
                    k => out.push((day, k, wh.max(0.0))),
                }
            }
        }
        out
    }
}

impl Energy {
    /// New daily records set by this packet (only those — nothing to write otherwise).
    pub fn extremes(&self, device: &str, ts: i64, metrics: &HashMap<String, f64>, tz: &Tz) -> Vec<(String, &'static str, f64)> {
        let mut s = self.s.lock().unwrap();
        let day = tz.day(ts);
        let mut out = Vec::new();
        for (key, dev, metric, is_max) in EXT {
            if dev != device {
                continue;
            }
            let Some(v) = metrics.get(metric).copied().filter(|v| v.is_finite()) else { continue };
            let k = (day.clone(), key);
            let better = match s.ext.get(&k) {
                None => true,
                Some(&old) => if is_max { v > old } else { v < old },
            };
            if better {
                s.ext.insert(k, v);
                out.push((day.clone(), key, v));
            }
        }
        // keep only the current day in memory
        if s.ext.len() > 16 {
            s.ext.retain(|(d, _), _| *d == day);
        }
        out
    }
}

pub fn store_ext(conn: &Connection, recs: &[(String, &'static str, f64)]) -> rusqlite::Result<()> {
    for (day, key, v) in recs {
        let sql = if key.ends_with("_min") {
            "INSERT INTO energy_ext(day, key, v) VALUES(?1, ?2, ?3) ON CONFLICT(day, key) DO UPDATE SET v = MIN(v, excluded.v)"
        } else {
            "INSERT INTO energy_ext(day, key, v) VALUES(?1, ?2, ?3) ON CONFLICT(day, key) DO UPDATE SET v = MAX(v, excluded.v)"
        };
        conn.prepare_cached(sql)?.execute(params![day, key, v])?;
    }
    Ok(())
}

/// One-time fill of daily extremes from stored history (best effort: older
/// history is already thinned, so its peaks are averages).
pub fn backfill_ext(conn: &Connection, tz: &Tz) -> rusqlite::Result<usize> {
    let empty: bool = conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM energy_ext)", [], |r| r.get(0))?;
    if !empty {
        return Ok(0);
    }
    let mut st = conn.prepare(
        "SELECT p.ts, m.device, m.metric, p.v FROM points p JOIN metrics m ON m.id = p.mid
         WHERE (m.device = 'inverter' AND m.metric IN ('pv_w', 'ac_out_w')) OR (m.device = 'bms' AND m.metric = 'soc')",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)? as f64 / 1000.0)))?;
    let e = Energy::new();
    let mut n = 0;
    for row in rows {
        let (ts, dev, metric, v) = row?;
        let recs = e.extremes(&dev, ts, &HashMap::from([(metric, v)]), tz);
        n += recs.len();
        store_ext(conn, &recs)?;
    }
    Ok(n)
}

pub fn store(conn: &Connection, deltas: &[(String, &'static str, f64)]) -> rusqlite::Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT INTO energy_daily(day, key, wh) VALUES(?1, ?2, ?3)
         ON CONFLICT(day, key) DO UPDATE SET wh = wh + excluded.wh",
    )?;
    for (day, key, wh) in deltas {
        if *wh > 0.0 {
            st.execute(params![day, key, wh])?;
        }
    }
    Ok(())
}

/// kWh per key for one day.
pub fn day_totals(conn: &Connection, day: &str) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    if let Ok(mut st) = conn.prepare_cached("SELECT key, wh FROM energy_daily WHERE day = ?1") {
        if let Ok(rows) = st.query_map(params![day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))) {
            for (k, wh) in rows.flatten() {
                m.insert(k, wh / 1000.0);
            }
        }
    }
    m
}

/// One-time rebuild from stored history (first run only): replay the four
/// source metrics in time order through the same integrator.
pub fn backfill(conn: &Connection, tz: &Tz) -> rusqlite::Result<usize> {
    let empty: bool = conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM energy_daily)", [], |r| r.get(0))?;
    if !empty {
        return Ok(0);
    }
    let mut ev: Vec<(i64, String, String, f64)> = {
        let mut st = conn.prepare(
            "SELECT p.ts, m.device, m.metric, p.v FROM points p JOIN metrics m ON m.id = p.mid
             WHERE (m.device = 'inverter' AND m.metric IN ('pv_w', 'ac_out_w', 'grid_v'))
                OR (m.device = 'bms' AND m.metric = 'bms_w')",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as f64 / 1000.0)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    ev.sort_by_key(|e| e.0);
    let e = Energy::new();
    let mut n = 0;
    for (ts, dev, metric, v) in ev {
        let d = e.feed(&dev, ts, &HashMap::from([(metric, v)]), tz);
        n += d.len();
        store(conn, &d)?;
    }
    Ok(n)
}

#[derive(Deserialize)]
pub struct Q {
    days: Option<i64>,
}

/// Daily kWh for the last `days` local days (default 31), newest last.
pub async fn api(State(st): State<AppState>, Query(q): Query<Q>) -> Response {
    let tz = st.config.get().timezone;
    let now = crate::now();
    let days = q.days.unwrap_or(31).clamp(1, 800);
    let from_day = tz.day(now - (days - 1) * 86_400);
    let rows: rusqlite::Result<Vec<(String, String, f64)>> = {
        let c = st.db.lock().unwrap();
        c.prepare("SELECT day, key, wh FROM energy_daily WHERE day >= ?1 ORDER BY day")
            .and_then(|mut s| s.query_map(params![from_day], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
    };
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let ext: Vec<(String, String, f64)> = {
        let c = st.db.lock().unwrap();
        c.prepare("SELECT day, key, v FROM energy_ext WHERE day >= ?1")
            .and_then(|mut s| s.query_map(params![from_day], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
            .unwrap_or_default()
    };
    let mut by: Vec<(String, serde_json::Map<String, serde_json::Value>)> = Vec::new();
    for (day, key, wh) in rows {
        if by.last().map(|(d, _)| d != &day).unwrap_or(true) {
            by.push((day.clone(), serde_json::Map::new()));
        }
        by.last_mut().unwrap().1.insert(key, serde_json::json!((wh / 10.0).round() / 100.0));
    }
    for (day, key, v) in ext {
        let i = match by.iter().position(|(d, _)| *d == day) {
            Some(i) => i,
            None => {
                let i = by.iter().position(|(d, _)| *d > day).unwrap_or(by.len());
                by.insert(i, (day.clone(), serde_json::Map::new()));
                i
            }
        };
        by[i].1.insert(key, serde_json::json!((v * 10.0).round() / 10.0));
    }
    let items: Vec<_> = by.into_iter().map(|(day, mut m)| { m.insert("day".into(), day.into()); serde_json::Value::Object(m) }).collect();
    Json(serde_json::json!({ "today": tz.day(now), "keys": KEYS, "days": items })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tz::days_from_civil;

    fn pk(dev: &str, k: &str, v: f64) -> (String, HashMap<String, f64>) {
        (dev.to_string(), HashMap::from([(k.to_string(), v)]))
    }

    #[test]
    fn integrates_power() {
        let e = Energy::new();
        let tz = Tz::default();
        let t0 = days_from_civil(2026, 7, 1) * 86_400; // 03:00 local
        let mut total = HashMap::<&str, f64>::new();
        for i in 0..=720 {
            // 1 kW load, 500 W PV, battery −400 W, grid up → grid ≈ 100 W, 1 hour in 5 s steps
            let ts = t0 + i * 5;
            for (dev, m) in [pk("bms", "bms_w", -400.0), {
                let mut m = HashMap::new();
                m.insert("pv_w".into(), 500.0); m.insert("ac_out_w".into(), 1000.0); m.insert("grid_v".into(), 230.0);
                ("inverter".to_string(), m)
            }] {
                for (_, k, wh) in e.feed(&dev, ts, &m, &tz) { *total.entry(k).or_default() += wh; }
            }
        }
        let near = |k: &str, want: f64| assert!((total.get(k).copied().unwrap_or(0.0) - want).abs() < 1.0, "{k}: {:?}", total.get(k));
        near("load", 1000.0);
        near("pv", 500.0);
        near("dis", 400.0);
        near("grid", 100.0);
        assert_eq!(total.get("chg"), None);
    }

    #[test]
    fn daily_extremes_only_on_records() {
        let e = Energy::new();
        let tz = Tz::default();
        let t = days_from_civil(2026, 7, 1) * 86_400 + 9 * 3600; // 12:00 local
        let m = |v: f64| HashMap::from([("soc".to_string(), v)]);
        assert_eq!(e.extremes("bms", t, &m(80.0), &tz).len(), 2, "first value is both min and max");
        assert!(e.extremes("bms", t + 5, &m(80.0), &tz).is_empty(), "no new record → nothing to write");
        assert_eq!(e.extremes("bms", t + 10, &m(75.0), &tz), vec![("2026-07-01".to_string(), "soc_min", 75.0)]);
        assert!(e.extremes("inverter", t, &m(75.0), &tz).is_empty(), "soc only counts from the bms");
    }

    #[test]
    fn gaps_and_days() {
        let e = Energy::new();
        let tz = Tz::default();
        let t = days_from_civil(2026, 12, 1) * 86_400 - 2 * 3600 - 10; // 23:59:50 local on 30 Nov
        let m = |w: f64| HashMap::from([("pv_w".to_string(), w)]);
        assert!(e.feed("inverter", t, &m(3600.0), &tz).is_empty());
        let d = e.feed("inverter", t + 10, &m(3600.0), &tz);
        assert_eq!(d, vec![("2026-11-30".to_string(), "pv", 10.0)], "10 s at 3.6 kW = 10 Wh, on the day it started");
        assert!(e.feed("inverter", t + 10 + 600, &m(0.0), &tz).is_empty(), "10-minute gap isn't integrated");
    }
}
