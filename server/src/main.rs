//! solarbox — a tiny push-based telemetry server.
//!
//! Boards POST their readings to `/api/ingest`; nothing is polled. Readings go
//! into SQLite; the embedded dashboard reads them back via `/api/latest`,
//! `/api/series` and `/api/catalog`. The whole thing is one static binary with
//! the UI baked in, meant to sit in a container on the site's router.

mod db;

use axum::{
    extract::{Json, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rusqlite::Connection;
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    /// Optional shared secret; when set, ingest requires header `X-Token`.
    token: Option<String>,
    /// Age→resolution tiers for downsampling, sorted by age.
    tiers: Arc<Vec<db::Tier>>,
    /// Lookback (secs) for the open-ended oldest tier.
    tail: i64,
}

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let db_path = env("SOLARBOX_DB", "solarbox.db");
    let addr = env("SOLARBOX_ADDR", "0.0.0.0:8080");
    // Hard delete is off by default — tiered compaction is the size control.
    let retention_days: i64 = env("SOLARBOX_RETENTION_DAYS", "0").parse().unwrap_or(0);
    let token = std::env::var("SOLARBOX_TOKEN").ok().filter(|s| !s.is_empty());
    let tiers: Vec<db::Tier> = std::env::var("SOLARBOX_TIERS")
        .ok()
        .and_then(|s| parse_tiers(&s))
        .unwrap_or_else(default_tiers);
    let compact_every = parse_dur(&env("SOLARBOX_COMPACT_EVERY", "6h")).unwrap_or(21_600).max(60);
    let tail = parse_dur(&env("SOLARBOX_COMPACT_TAIL", "2d")).unwrap_or(172_800);

    let conn = db::open(&db_path).expect("open database");
    let db = Arc::new(Mutex::new(conn));
    let tiers = Arc::new(tiers);
    let state = AppState {
        db: db.clone(),
        token: token.clone(),
        tiers: tiers.clone(),
        tail,
    };

    // Background maintenance: tiered downsampling, then optional hard retention.
    {
        let mdb = db.clone();
        let mtiers = tiers.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await; // let startup settle
            loop {
                {
                    let mut c = mdb.lock().unwrap();
                    match db::compact(&mut c, &mtiers, now(), tail) {
                        Ok(n) if n > 0 => eprintln!("[compact] rewrote {n} rows"),
                        Ok(_) => {}
                        Err(e) => eprintln!("[compact] {e}"),
                    }
                    if retention_days > 0 {
                        let cutoff = now() - retention_days * 86_400;
                        if let Ok(n) = db::prune(&c, cutoff) {
                            if n > 0 {
                                eprintln!("[retention] removed {n} rows");
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(compact_every as u64)).await;
            }
        });
    }

    let app = Router::new()
        .route("/api/ingest", post(ingest))
        .route("/api/latest", get(latest))
        .route("/api/series", get(series))
        .route("/api/catalog", get(catalog))
        .route("/api/compact", post(compact_now))
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(|| async { serve("index.html") }))
        .route("/*path", get(asset))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind address");
    eprintln!(
        "solarbox on {addr}  db={db_path}  retention={retention_days}d  auth={}",
        if token.is_some() { "token" } else { "open" }
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .unwrap();
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

// ---- ingest ---------------------------------------------------------------

#[derive(Deserialize)]
struct Ingest {
    device: String,
    ts: Option<i64>,
    metrics: HashMap<String, f64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IngestBody {
    One(Ingest),
    Many(Vec<Ingest>),
}

async fn ingest(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<IngestBody>) -> Response {
    if let Some(tok) = &st.token {
        let ok = headers
            .get("x-token")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == tok)
            .unwrap_or(false);
        if !ok {
            return (StatusCode::UNAUTHORIZED, "bad token").into_response();
        }
    }

    let items = match body {
        IngestBody::One(x) => vec![x],
        IngestBody::Many(v) => v,
    };

    let mut stored = 0usize;
    {
        let mut c = st.db.lock().unwrap();
        for it in &items {
            let ts = it.ts.unwrap_or_else(now);
            match db::insert(&mut c, &it.device, ts, &it.metrics) {
                Ok(n) => stored += n,
                Err(e) => {
                    eprintln!("[ingest] {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
                }
            }
        }
    }
    Json(serde_json::json!({ "ok": true, "stored": stored })).into_response()
}

/// Force a compaction pass now (same work the background task does).
async fn compact_now(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(tok) = &st.token {
        let ok = headers
            .get("x-token")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == tok)
            .unwrap_or(false);
        if !ok {
            return (StatusCode::UNAUTHORIZED, "bad token").into_response();
        }
    }
    let res = {
        let mut c = st.db.lock().unwrap();
        db::compact(&mut c, &st.tiers, now(), st.tail)
    };
    match res {
        Ok(n) => Json(serde_json::json!({ "ok": true, "rewrote": n })).into_response(),
        Err(e) => {
            eprintln!("[compact] {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

// ---- tier config parsing --------------------------------------------------

/// Parse a duration like `20`, `20s`, `5m`, `6h`, `2d` into seconds.
fn parse_dur(s: &str) -> Option<i64> {
    let s = s.trim();
    let last = s.chars().last()?;
    let (num, mul) = match last {
        's' => (&s[..s.len() - 1], 1),
        'm' => (&s[..s.len() - 1], 60),
        'h' => (&s[..s.len() - 1], 3600),
        'd' => (&s[..s.len() - 1], 86_400),
        _ => (s, 1),
    };
    num.trim().parse::<i64>().ok().map(|n| n * mul)
}

/// Default scheme: today 20s, then 1m, 5m, 10m, 15m, 20m, 25m by day, cap 30m.
fn default_tiers() -> Vec<db::Tier> {
    [
        (0, 20),
        (86_400, 60),
        (172_800, 300),
        (259_200, 600),
        (345_600, 900),
        (432_000, 1200),
        (518_400, 1500),
        (604_800, 1800),
    ]
    .iter()
    .map(|&(min_age, res)| db::Tier { min_age, res })
    .collect()
}

/// Parse `SOLARBOX_TIERS="0:20s,1d:1m,2d:5m,..."` (age:resolution, comma-sep).
fn parse_tiers(s: &str) -> Option<Vec<db::Tier>> {
    let mut v = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, r) = part.split_once(':')?;
        v.push(db::Tier {
            min_age: parse_dur(a)?,
            res: parse_dur(r)?,
        });
    }
    if v.is_empty() {
        return None;
    }
    v.sort_by_key(|t| t.min_age);
    Some(v)
}

// ---- reads ----------------------------------------------------------------

async fn latest(State(st): State<AppState>) -> Response {
    let rows = {
        let c = st.db.lock().unwrap();
        db::catalog(&c)
    };
    match rows {
        Ok(rows) => {
            let mut out = serde_json::Map::new();
            for (device, metric, value, ts) in rows {
                let dev = out
                    .entry(device)
                    .or_insert_with(|| serde_json::json!({}))
                    .as_object_mut()
                    .unwrap();
                dev.insert(metric, serde_json::json!({ "v": value, "ts": ts }));
            }
            Json(serde_json::Value::Object(out)).into_response()
        }
        Err(e) => {
            eprintln!("[latest] {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn catalog(State(st): State<AppState>) -> Response {
    let rows = {
        let c = st.db.lock().unwrap();
        db::catalog(&c)
    };
    match rows {
        Ok(rows) => {
            let list: Vec<_> = rows
                .into_iter()
                .map(|(d, m, _v, ts)| serde_json::json!({ "device": d, "metric": m, "ts": ts }))
                .collect();
            Json(serde_json::json!({ "items": list })).into_response()
        }
        Err(e) => {
            eprintln!("[catalog] {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

#[derive(Deserialize)]
struct SeriesQ {
    device: String,
    metric: String,
    from: Option<i64>,
    to: Option<i64>,
    points: Option<i64>,
}

async fn series(State(st): State<AppState>, Query(q): Query<SeriesQ>) -> Response {
    let to = q.to.unwrap_or_else(now);
    let from = q.from.unwrap_or(to - 86_400);
    let pts = q.points.unwrap_or(600).clamp(10, 5000);
    let res = {
        let c = st.db.lock().unwrap();
        db::series(&c, &q.device, &q.metric, from, to, pts)
    };
    match res {
        Ok((ts, v)) => Json(serde_json::json!({ "ts": ts, "v": v })).into_response(),
        Err(e) => {
            eprintln!("[series] {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

// ---- static dashboard (embedded) ------------------------------------------

async fn asset(Path(path): Path<String>) -> Response {
    serve(&path)
}

fn serve(path: &str) -> Response {
    match Assets::get(path) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref().to_string())], f.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("sigterm");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    eprintln!("shutting down");
}
