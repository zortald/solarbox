//! Power outage tracking. The inverter's grid voltage is watched on every
//! pushed packet: an outage starts when it stays below `LOW` for `HOLD_S` and
//! ends when it stays above `HIGH` for `HOLD_S` (hysteresis + hold so a dip or
//! a single bad reading doesn't count). Outages go into their own small table
//! so compaction of the history never blurs them. A silent board is not an
//! outage — the state simply waits for the next reading.

use crate::AppState;
use axum::{extract::{Query, State}, response::{IntoResponse, Response}, Json};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::sync::Mutex;

pub const LOW: f64 = 150.0;
pub const HIGH: f64 = 180.0;
pub const HOLD_S: i64 = 10;

#[derive(Default)]
struct State_ {
    open: Option<(i64, i64)>,  // (row id, start ts) of the current outage
    below_since: Option<i64>,  // first low reading while grid is up
    above_since: Option<i64>,  // first good reading while in an outage
}

pub struct Detector {
    s: Mutex<State_>,
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS outages(
             id        INTEGER PRIMARY KEY,
             start     INTEGER NOT NULL,
             end       INTEGER,
             soc_start REAL,
             soc_end   REAL
         );
         CREATE INDEX IF NOT EXISTS idx_outages_start ON outages(start);",
    )
}

fn soc_at(conn: &Connection, ts: i64) -> Option<f64> {
    conn.query_row(
        "SELECT p.v FROM points p JOIN metrics m ON m.id = p.mid
         WHERE m.device = 'bms' AND m.metric = 'soc' AND p.ts <= ?1 ORDER BY p.ts DESC LIMIT 1",
        params![ts],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|v| v as f64 / 1000.0)
}

impl Detector {
    /// Restore the open outage (if the server restarted during one) and, on
    /// the very first run, rebuild past outages from stored grid voltage.
    pub fn open(conn: &Connection) -> rusqlite::Result<Self> {
        init(conn)?;
        let d = Detector { s: Mutex::new(State_::default()) };
        let empty: bool = conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM outages)", [], |r| r.get(0))?;
        if empty {
            let pts: Vec<(i64, i64)> = {
                let mut st = conn.prepare(
                    "SELECT p.ts, p.v FROM points p JOIN metrics m ON m.id = p.mid
                     WHERE m.device = 'inverter' AND m.metric = 'grid_v' ORDER BY p.ts",
                )?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (ts, v) in &pts {
                d.feed(conn, *ts, *v as f64 / 1000.0, soc_at(conn, *ts))?;
            }
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM outages", [], |r| r.get(0))?;
            if !pts.is_empty() {
                eprintln!("[outages] rebuilt {n} outage(s) from {} grid readings", pts.len());
            }
        }
        let open: Option<(i64, i64)> = conn
            .query_row("SELECT id, start FROM outages WHERE end IS NULL ORDER BY start DESC LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        d.s.lock().unwrap().open = open;
        Ok(d)
    }

    /// One grid-voltage reading. Returns Some(true) when an outage started,
    /// Some(false) when one ended.
    pub fn feed(&self, conn: &Connection, ts: i64, grid_v: f64, soc: Option<f64>) -> rusqlite::Result<Option<bool>> {
        let mut s = self.s.lock().unwrap();
        match s.open {
            None => {
                if grid_v < LOW {
                    let since = *s.below_since.get_or_insert(ts);
                    if ts - since >= HOLD_S {
                        conn.execute("INSERT INTO outages(start, soc_start) VALUES(?1, ?2)", params![since, soc])?;
                        s.open = Some((conn.last_insert_rowid(), since));
                        s.below_since = None;
                        return Ok(Some(true));
                    }
                } else {
                    s.below_since = None;
                }
            }
            Some((id, _)) => {
                if grid_v > HIGH {
                    let since = *s.above_since.get_or_insert(ts);
                    if ts - since >= HOLD_S {
                        conn.execute("UPDATE outages SET end = ?1, soc_end = ?2 WHERE id = ?3", params![since, soc, id])?;
                        s.open = None;
                        s.above_since = None;
                        return Ok(Some(false));
                    }
                } else {
                    s.above_since = None;
                }
            }
        }
        Ok(None)
    }

    pub fn current_start(&self) -> Option<i64> {
        self.s.lock().unwrap().open.map(|(_, s)| s)
    }
}

/// Import past outages (e.g. from the inverter board's own log) as
/// `[[start, secs], ...]`; ones overlapping an existing outage are skipped.
pub fn import(conn: &Connection, items: &[(i64, i64)]) -> rusqlite::Result<usize> {
    let mut added = 0;
    for &(start, secs) in items {
        if start <= 0 || secs <= 0 {
            continue;
        }
        let end = start + secs;
        let clash: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM outages WHERE start <= ?2 AND COALESCE(end, ?2) >= ?1)",
            params![start, end],
            |r| r.get(0),
        )?;
        if clash {
            continue;
        }
        conn.execute(
            "INSERT INTO outages(start, end, soc_start, soc_end) VALUES(?1, ?2, ?3, ?4)",
            params![start, end, soc_at(conn, start), soc_at(conn, end)],
        )?;
        added += 1;
    }
    Ok(added)
}

#[derive(Deserialize)]
pub struct Q {
    from: Option<i64>,
    to: Option<i64>,
    /// Local midnight from the browser (the container has no timezone data).
    midnight: Option<i64>,
}

/// Outages overlapping [from, to] plus summary windows (today, 7 d, 30 d).
pub async fn list(State(st): State<AppState>, Query(q): Query<Q>) -> Response {
    let now = crate::now();
    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(now - 30 * 86_400);
    let midnight = q.midnight.unwrap_or(now - now.rem_euclid(86_400));
    let rows: Vec<(i64, Option<i64>, Option<f64>, Option<f64>)> = {
        let c = st.db.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT start, end, soc_start, soc_end FROM outages
             WHERE start <= ?2 AND (end IS NULL OR end >= ?1) ORDER BY start DESC",
        ) {
            Ok(s) => s,
            Err(e) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        let lo = from.min(now - 30 * 86_400);
        let r = stmt
            .query_map(params![lo, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .and_then(|it| it.collect());
        match r {
            Ok(v) => v,
            Err(e) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    };
    let window = |a: i64, b: i64| {
        let mut n = 0;
        let mut secs = 0;
        let mut longest = 0;
        for (s, e, _, _) in &rows {
            let e = e.unwrap_or(now);
            let ov = e.min(b) - (*s).max(a);
            if ov > 0 || (*s >= a && *s <= b) {
                n += 1;
                secs += ov.max(0);
                longest = longest.max(e - s);
            }
        }
        serde_json::json!({ "count": n, "secs": secs, "longest": longest })
    };
    let items: Vec<_> = rows
        .iter()
        .filter(|(s, e, _, _)| *s <= to && e.unwrap_or(now) >= from)
        .take(100)
        .map(|(s, e, a, b)| serde_json::json!({ "start": s, "end": e, "secs": e.unwrap_or(now) - s, "soc_start": a, "soc_end": b }))
        .collect();
    Json(serde_json::json!({
        "current": st.outages.current_start().map(|s| serde_json::json!({ "start": s, "secs": now - s })),
        "today": window(midnight, now),
        "d7": window(now - 7 * 86_400, now),
        "d30": window(now - 30 * 86_400, now),
        "items": items,
        "thresholds": { "low": LOW, "high": HIGH, "hold_s": HOLD_S },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_skips_overlaps() {
        let c = conn();
        assert_eq!(import(&c, &[(1000, 30), (5000, 20)]).unwrap(), 2);
        assert_eq!(import(&c, &[(1010, 5), (5000, 20), (9000, 10)]).unwrap(), 1, "only the new one");
    }

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::db::init(&c).unwrap();
        init(&c).unwrap();
        c
    }

    #[test]
    fn hysteresis_and_hold() {
        let c = conn();
        let d = Detector::open(&c).unwrap();
        assert_eq!(d.feed(&c, 100, 230.0, Some(90.0)).unwrap(), None);
        assert_eq!(d.feed(&c, 105, 0.0, Some(90.0)).unwrap(), None, "a single low reading isn't an outage");
        assert_eq!(d.feed(&c, 110, 230.0, Some(90.0)).unwrap(), None, "dip recovered");
        assert_eq!(d.feed(&c, 200, 0.0, Some(89.0)).unwrap(), None);
        assert_eq!(d.feed(&c, 210, 0.0, Some(89.0)).unwrap(), Some(true), "held 10 s → outage from 200");
        assert_eq!(d.current_start(), Some(200));
        assert_eq!(d.feed(&c, 300, 170.0, Some(80.0)).unwrap(), None, "170 V is not 'back' yet (hysteresis)");
        assert_eq!(d.feed(&c, 400, 231.0, Some(75.0)).unwrap(), None);
        assert_eq!(d.feed(&c, 410, 231.0, Some(75.0)).unwrap(), Some(false), "back from 400");
        let (s, e, a, b): (i64, i64, f64, f64) =
            c.query_row("SELECT start, end, soc_start, soc_end FROM outages", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!((s, e, a, b), (200, 400, 89.0, 75.0));
    }

    #[test]
    fn survives_restart_and_rebuilds() {
        let c = conn();
        let put = |ts: i64, v: f64| {
            let id: i64 = { c.execute("INSERT OR IGNORE INTO metrics(device, metric) VALUES('inverter','grid_v')", []).unwrap();
                c.query_row("SELECT id FROM metrics WHERE device='inverter' AND metric='grid_v'", [], |r| r.get(0)).unwrap() };
            c.execute("INSERT INTO points(mid, ts, v) VALUES(?1, ?2, ?3)", params![id, ts, (v * 1000.0) as i64]).unwrap();
        };
        put(0, 230.0);
        put(60, 0.0);
        put(120, 0.0); // outage from 60, still open
        let d = Detector::open(&c).unwrap();
        assert_eq!(d.current_start(), Some(60), "rebuilt from history and left open");
        let d2 = Detector::open(&c).unwrap();
        assert_eq!(d2.current_start(), Some(60), "open outage restored after restart");
    }
}
