//! SQLite storage: a long time-series table plus a tiny "latest" table that is
//! upserted on every ingest so the dashboard's "now" view is a trivial read.

use rusqlite::{params, Connection};
use std::collections::HashMap;

/// Open (or create) the database and apply schema + pragmas suited to a router:
/// WAL for concurrent read while writing, NORMAL sync (safe enough on flash),
/// and a busy timeout so a dashboard read never errors out against an ingest.
pub fn open(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS samples(
             ts     INTEGER NOT NULL,
             device TEXT    NOT NULL,
             metric TEXT    NOT NULL,
             value  REAL    NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_samples_q  ON samples(device, metric, ts);
         CREATE INDEX IF NOT EXISTS idx_samples_ts ON samples(ts);
         CREATE TABLE IF NOT EXISTS latest(
             device TEXT    NOT NULL,
             metric TEXT    NOT NULL,
             value  REAL    NOT NULL,
             ts     INTEGER NOT NULL,
             PRIMARY KEY(device, metric)
         );",
    )?;
    Ok(conn)
}

/// Append one row per metric to `samples` and update `latest` (only forward in
/// time). Everything runs in a single transaction so a batch is atomic.
pub fn insert(
    conn: &mut Connection,
    device: &str,
    ts: i64,
    metrics: &HashMap<String, f64>,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    {
        let mut s = tx.prepare_cached(
            "INSERT INTO samples(ts, device, metric, value) VALUES(?1, ?2, ?3, ?4)",
        )?;
        let mut l = tx.prepare_cached(
            "INSERT INTO latest(device, metric, value, ts) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(device, metric) DO UPDATE SET value = excluded.value, ts = excluded.ts
             WHERE excluded.ts >= latest.ts",
        )?;
        for (metric, value) in metrics {
            s.execute(params![ts, device, metric, value])?;
            l.execute(params![device, metric, value, ts])?;
        }
    }
    let n = metrics.len();
    tx.commit()?;
    Ok(n)
}

/// Latest value + timestamp for every (device, metric) pair seen.
pub fn catalog(conn: &Connection) -> rusqlite::Result<Vec<(String, String, f64, i64)>> {
    let mut st =
        conn.prepare_cached("SELECT device, metric, value, ts FROM latest ORDER BY device, metric")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}

/// Time series for one metric between `from` and `to` (unix seconds). When the
/// window is wide relative to `maxpoints`, values are averaged into fixed-width
/// time buckets so the payload stays small regardless of range.
pub fn series(
    conn: &Connection,
    device: &str,
    metric: &str,
    from: i64,
    to: i64,
    maxpoints: i64,
) -> rusqlite::Result<(Vec<i64>, Vec<f64>)> {
    let span = (to - from).max(1);
    let bucket = (span / maxpoints.max(1)).max(1);
    let mut ts = Vec::new();
    let mut v = Vec::new();
    if bucket <= 1 {
        let mut st = conn.prepare_cached(
            "SELECT ts, value FROM samples
             WHERE device = ?1 AND metric = ?2 AND ts >= ?3 AND ts <= ?4
             ORDER BY ts",
        )?;
        let rows = st.query_map(params![device, metric, from, to], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?;
        for row in rows {
            let (t, x) = row?;
            ts.push(t);
            v.push(x);
        }
    } else {
        let mut st = conn.prepare_cached(
            "SELECT (ts / ?5) * ?5 AS b, AVG(value) FROM samples
             WHERE device = ?1 AND metric = ?2 AND ts >= ?3 AND ts <= ?4
             GROUP BY b ORDER BY b",
        )?;
        let rows = st.query_map(params![device, metric, from, to, bucket], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?;
        for row in rows {
            let (t, x) = row?;
            ts.push(t);
            v.push(x);
        }
    }
    Ok((ts, v))
}

/// Delete samples older than `cutoff` (unix seconds). `latest` is untouched.
pub fn prune(conn: &Connection, cutoff: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM samples WHERE ts < ?1", params![cutoff])
}
