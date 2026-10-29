//! SQLite storage, sized for a router's flash.
//!
//! * `metrics` — dictionary (device, metric) → small integer id.
//! * `points`  — (mid, ts, v) WITHOUT ROWID, primary key (mid, ts); the value is
//!   stored as an integer in thousandths (27.637 → 27637), so a row costs a few
//!   varint bytes instead of two text columns and two indexes.
//! * `latest`  — newest value per (device, metric) for the "now" view.
//!
//! Unchanged values aren't re-stored: a point is written when the value
//! changes or at least every `HEARTBEAT_S`. WAL is size-limited and freed pages
//! are returned to the filesystem (incremental auto_vacuum). A size cap drops
//! the oldest day if the file ever grows past it.

use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// Re-store an unchanged value at least this often (keeps charts/energy sane).
pub const HEARTBEAT_S: i64 = 60;
/// Metrics kept in `latest` only, never in history.
const NO_HISTORY: [&str; 1] = ["uptime"];
const SCALE: f64 = 1000.0;

pub fn open(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    // auto_vacuum must be chosen before the first table exists (or via VACUUM).
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "journal_size_limit", 1_048_576)?; // truncate WAL to 1 MB after checkpoints
    conn.pragma_update(None, "wal_autocheckpoint", 256)?; // ~1 MB of pages
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metrics(
             id     INTEGER PRIMARY KEY,
             device TEXT NOT NULL,
             metric TEXT NOT NULL,
             UNIQUE(device, metric)
         );
         CREATE TABLE IF NOT EXISTS points(
             mid INTEGER NOT NULL,
             ts  INTEGER NOT NULL,
             v   INTEGER NOT NULL,
             PRIMARY KEY(mid, ts)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS latest(
             device TEXT    NOT NULL,
             metric TEXT    NOT NULL,
             value  REAL    NOT NULL,
             ts     INTEGER NOT NULL,
             PRIMARY KEY(device, metric)
         );",
    )?;
    migrate_v1(conn)?;
    // A database created before auto_vacuum was enabled needs one VACUUM.
    let av: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if av != 2 {
        eprintln!("[db] enabling incremental auto_vacuum (one-time VACUUM)");
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
    }
    Ok(())
}

/// One-time move from the original long-format `samples` table.
fn migrate_v1(conn: &Connection) -> rusqlite::Result<()> {
    let old: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='samples')",
        [],
        |r| r.get(0),
    )?;
    if !old {
        return Ok(());
    }
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))?;
    eprintln!("[db] migrating {n} samples to the compact format");
    conn.execute_batch(
        "BEGIN;
         INSERT OR IGNORE INTO metrics(device, metric) SELECT DISTINCT device, metric FROM samples;
         INSERT OR REPLACE INTO points(mid, ts, v)
           SELECT m.id, s.ts, CAST(ROUND(s.value * 1000) AS INTEGER)
           FROM samples s JOIN metrics m ON m.device = s.device AND m.metric = s.metric;
         DROP TABLE samples;
         COMMIT;
         VACUUM;",
    )?;
    let m: i64 = conn.query_row("SELECT COUNT(*) FROM points", [], |r| r.get(0))?;
    eprintln!("[db] migration done: {m} points");
    Ok(())
}

fn mid(conn: &Connection, device: &str, metric: &str) -> rusqlite::Result<i64> {
    conn.prepare_cached("INSERT OR IGNORE INTO metrics(device, metric) VALUES(?1, ?2)")?
        .execute(params![device, metric])?;
    conn.prepare_cached("SELECT id FROM metrics WHERE device = ?1 AND metric = ?2")?
        .query_row(params![device, metric], |r| r.get(0))
}

fn find_mid(conn: &Connection, device: &str, metric: &str) -> rusqlite::Result<Option<i64>> {
    conn.prepare_cached("SELECT id FROM metrics WHERE device = ?1 AND metric = ?2")?
        .query_row(params![device, metric], |r| r.get(0))
        .optional()
}

/// Store one packet: always update `latest`; append to history only when the
/// value changed or the heartbeat is due. Returns the number of metrics seen.
pub fn insert(conn: &mut Connection, device: &str, ts: i64, metrics: &HashMap<String, f64>) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    for (metric, value) in metrics {
        tx.prepare_cached(
            "INSERT INTO latest(device, metric, value, ts) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(device, metric) DO UPDATE SET value = excluded.value, ts = excluded.ts
             WHERE excluded.ts >= latest.ts",
        )?
        .execute(params![device, metric, value, ts])?;
        if NO_HISTORY.contains(&metric.as_str()) || !value.is_finite() {
            continue;
        }
        let id = mid(&tx, device, metric)?;
        let v = (value * SCALE).round() as i64;
        let last: Option<(i64, i64)> = tx
            .prepare_cached("SELECT ts, v FROM points WHERE mid = ?1 ORDER BY ts DESC LIMIT 1")?
            .query_row(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let store = match last {
            None => true,
            Some((lts, lv)) => lv != v || ts - lts >= HEARTBEAT_S || ts < lts,
        };
        if store {
            tx.prepare_cached("INSERT OR REPLACE INTO points(mid, ts, v) VALUES(?1, ?2, ?3)")?
                .execute(params![id, ts, v])?;
        }
    }
    let n = metrics.len();
    tx.commit()?;
    Ok(n)
}

/// Latest value + timestamp for every (device, metric) pair seen.
pub fn catalog(conn: &Connection) -> rusqlite::Result<Vec<(String, String, f64, i64)>> {
    let mut st = conn.prepare_cached("SELECT device, metric, value, ts FROM latest ORDER BY device, metric")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}

/// Time series for one metric between `from` and `to`, bucket-averaged when the
/// window is wide relative to `maxpoints`.
pub fn series(conn: &Connection, device: &str, metric: &str, from: i64, to: i64, maxpoints: i64) -> rusqlite::Result<(Vec<i64>, Vec<f64>)> {
    let Some(id) = find_mid(conn, device, metric)? else { return Ok((vec![], vec![])) };
    // Size buckets by where data actually exists in the window, not by the
    // requested window: 7 h of history in a "7 days" view stays full-resolution.
    let span: (Option<i64>, Option<i64>) = conn
        .prepare_cached("SELECT MIN(ts), MAX(ts) FROM points WHERE mid = ?1 AND ts >= ?2 AND ts <= ?3")?
        .query_row(params![id, from, to], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let (Some(from), Some(to)) = span else { return Ok((vec![], vec![])) };
    let bucket = ((to - from).max(1) / maxpoints.max(1)).max(1);
    let mut st = if bucket <= 1 {
        conn.prepare_cached("SELECT ts, CAST(v AS REAL) FROM points WHERE mid = ?1 AND ts >= ?2 AND ts <= ?3 AND ?4 > 0 ORDER BY ts")?
    } else {
        conn.prepare_cached(
            "SELECT (ts / ?4) * ?4 AS b, AVG(v) FROM points WHERE mid = ?1 AND ts >= ?2 AND ts <= ?3 GROUP BY b ORDER BY b",
        )?
    };
    let rows = st.query_map(params![id, from, to, bucket], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?;
    let (mut ts, mut v) = (Vec::new(), Vec::new());
    for row in rows {
        let (t, x) = row?;
        ts.push(t);
        v.push(x / SCALE);
    }
    Ok((ts, v))
}

/// Delete history older than `cutoff`.
pub fn prune(conn: &Connection, cutoff: i64) -> rusqlite::Result<usize> {
    let n = conn.execute("DELETE FROM points WHERE ts < ?1", params![cutoff])?;
    conn.execute_batch("PRAGMA incremental_vacuum;")?;
    Ok(n)
}

/// One age tier: samples at least `min_age` seconds old are stored at `res`
/// (seconds) resolution. Tiers are sorted ascending by `min_age`.
#[derive(Clone, Debug)]
pub struct Tier {
    pub min_age: i64,
    pub res: i64,
}

/// Thin history by age: within each tier's band, average points into `res`-s
/// buckets. Works per metric along the primary key; idempotent (bands already
/// on their grid are skipped). Frees the released pages afterwards.
pub fn compact(conn: &mut Connection, tiers: &[Tier], now: i64, tail: i64) -> rusqlite::Result<usize> {
    let mids: Vec<i64> = {
        let mut st = conn.prepare("SELECT id FROM metrics")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut total = 0usize;
    for (i, t) in tiers.iter().enumerate() {
        if t.res <= 1 {
            continue;
        }
        let hi = now - t.min_age;
        let lo = if i + 1 < tiers.len() { now - tiers[i + 1].min_age } else { now - (t.min_age + tail.max(0)) };
        if lo >= hi {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute("CREATE TEMP TABLE IF NOT EXISTS _roll(bts INTEGER PRIMARY KEY, v INTEGER)", [])?;
        for &id in &mids {
            let needs: bool = tx
                .prepare_cached("SELECT EXISTS(SELECT 1 FROM points WHERE mid = ?1 AND ts > ?2 AND ts <= ?3 AND ts % ?4 != 0)")?
                .query_row(params![id, lo, hi, t.res], |r| r.get(0))?;
            if !needs {
                continue;
            }
            tx.execute("DELETE FROM _roll", [])?;
            tx.prepare_cached(
                "INSERT INTO _roll SELECT (ts / ?4) * ?4, CAST(ROUND(AVG(v)) AS INTEGER)
                 FROM points WHERE mid = ?1 AND ts > ?2 AND ts <= ?3 GROUP BY 1",
            )?
            .execute(params![id, lo, hi, t.res])?;
            tx.prepare_cached("DELETE FROM points WHERE mid = ?1 AND ts > ?2 AND ts <= ?3")?
                .execute(params![id, lo, hi])?;
            total += tx
                .prepare_cached("INSERT OR REPLACE INTO points(mid, ts, v) SELECT ?1, bts, v FROM _roll")?
                .execute(params![id])?;
        }
        tx.commit()?;
    }
    conn.execute_batch("PRAGMA incremental_vacuum;")?;
    Ok(total)
}

/// Bytes the database actually uses (excluding free pages).
pub fn used_bytes(conn: &Connection) -> rusqlite::Result<i64> {
    let pc: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let fl: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let ps: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    Ok((pc - fl) * ps)
}

/// Safety net: while over `max_bytes`, drop the oldest day of history.
pub fn enforce_cap(conn: &Connection, max_bytes: i64) -> rusqlite::Result<usize> {
    let mut dropped = 0;
    for _ in 0..60 {
        if used_bytes(conn)? <= max_bytes {
            break;
        }
        let oldest: Option<i64> = conn.query_row("SELECT MIN(ts) FROM points", [], |r| r.get(0))?;
        let Some(o) = oldest else { break };
        dropped += conn.execute("DELETE FROM points WHERE ts < ?1", params![o + 86_400])?;
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
    }
    Ok(dropped)
}

/// Newest push timestamp per device.
pub fn last_push(conn: &Connection) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st = conn.prepare_cached("SELECT device, MAX(ts) FROM latest GROUP BY device")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// (points, devices, oldest ts, newest ts).
pub fn stats(conn: &Connection) -> rusqlite::Result<(i64, i64, Option<i64>, Option<i64>)> {
    let (n, a, b) = conn.query_row("SELECT COUNT(*), MIN(ts), MAX(ts) FROM points", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    let d: i64 = conn.query_row("SELECT COUNT(DISTINCT device) FROM metrics", [], |r| r.get(0))?;
    Ok((n, d, a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        c
    }
    fn put(c: &mut Connection, ts: i64, k: &str, v: f64) {
        insert(c, "bms", ts, &HashMap::from([(k.to_string(), v)])).unwrap();
    }
    fn count(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM points", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn dedupe_and_heartbeat() {
        let mut c = mem();
        for i in 0..12 {
            put(&mut c, 1000 + i * 5, "r1", 0.049); // unchanged for 55 s
        }
        assert_eq!(count(&c), 1, "unchanged values aren't re-stored");
        put(&mut c, 1060, "r1", 0.049);
        assert_eq!(count(&c), 2, "heartbeat after 60 s");
        put(&mut c, 1065, "r1", 0.050);
        assert_eq!(count(&c), 3, "a change is stored at once");
        put(&mut c, 1070, "uptime", 123.0);
        assert_eq!(count(&c), 3, "uptime is latest-only");
        assert_eq!(catalog(&c).unwrap().len(), 2);
    }

    #[test]
    fn values_roundtrip() {
        let mut c = mem();
        put(&mut c, 100, "bms_v", 27.637);
        put(&mut c, 105, "bms_v", -692.2892);
        let (ts, v) = series(&c, "bms", "bms_v", 0, 200, 1000).unwrap();
        assert_eq!(ts, vec![100, 105]);
        assert!((v[0] - 27.637).abs() < 1e-9 && (v[1] + 692.289).abs() < 1e-9);
        assert_eq!(series(&c, "bms", "nope", 0, 200, 10).unwrap().0.len(), 0);
    }

    #[test]
    fn buckets_follow_available_data() {
        let mut c = mem();
        for i in 0..400 {
            put(&mut c, 1_000_000 + i * 20, "soc", (i % 50) as f64);
        }
        // Ask for a week ending at the last point: only ~2 h exist → no coarse bucketing.
        let (ts, _) = series(&c, "bms", "soc", 1_000_000 + 400 * 20 - 604_800, 1_000_000 + 400 * 20, 600).unwrap();
        assert_eq!(ts.len(), 400);
    }

    #[test]
    fn compaction() {
        let mut c = mem();
        let now = 1_000_000;
        for i in 0..60 {
            put(&mut c, now - 2 * 86_400 + 100 + i * 5, "soc", (i % 7) as f64); // changes every 5 s
        }
        let before = count(&c);
        let tiers = vec![Tier { min_age: 0, res: 20 }, Tier { min_age: 86_400, res: 60 }, Tier { min_age: 172_800, res: 300 }];
        compact(&mut c, &tiers, now, 172_800).unwrap();
        let after = count(&c);
        assert!(after < before / 5, "{before} → {after}");
        assert_eq!(compact(&mut c, &tiers, now, 172_800).unwrap(), 0, "idempotent");
    }

    #[test]
    fn migration() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE samples(ts INTEGER, device TEXT, metric TEXT, value REAL);
             INSERT INTO samples VALUES(1,'bms','soc',92.0),(2,'bms','soc',93.0),(1,'inverter','pv_w',10.5);",
        )
        .unwrap();
        init(&c).unwrap();
        assert_eq!(count(&c), 3);
        let gone: bool = c
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='samples')", [], |r| r.get(0))
            .unwrap();
        assert!(!gone);
        let (_, v) = series(&c, "inverter", "pv_w", 0, 10, 100).unwrap();
        assert_eq!(v, vec![10.5]);
    }

    #[test]
    fn cap() {
        let mut c = mem();
        for d in 0..5 {
            for i in 0..2000 {
                put(&mut c, d * 86_400 + i * 30, "soc", i as f64);
            }
        }
        let before = used_bytes(&c).unwrap();
        enforce_cap(&c, before / 2).unwrap();
        assert!(used_bytes(&c).unwrap() <= before / 2);
        let oldest: i64 = c.query_row("SELECT MIN(ts) FROM points", [], |r| r.get(0)).unwrap();
        assert!(oldest >= 86_400, "oldest days go first");
    }
}
