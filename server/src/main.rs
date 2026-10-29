//! solarbox — a tiny push-based telemetry server.
//!
//! Boards POST their readings to `/api/ingest`; nothing is polled. Readings go
//! into SQLite; the embedded dashboard reads them back via `/api/latest`,
//! `/api/series` and `/api/catalog`. The whole thing is one static binary with
//! the UI baked in, meant to sit in a container on the site's router.

mod automations;
mod boards;
mod config;
mod db;
mod files;
mod ota;
mod sysinfo;
mod telegram;

use axum::{
    extract::{DefaultBodyLimit, Json, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rusqlite::Connection;
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
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
    /// Hot-reloaded config.json: boards, control rules, UI metadata.
    config: Arc<config::Store>,
    /// Live UI overrides (served before the embedded files).
    static_dir: PathBuf,
    /// Password for anything that changes state (None = control disabled).
    admin: Option<String>,
    ota_busy: Arc<AtomicBool>,
    /// CPU sampler + process start time for /api/system.
    sys: Arc<sysinfo::Sampler>,
    db_path: String,
    retention_days: i64,
    /// Every ingested packet is re-broadcast to open dashboards (SSE).
    bus: tokio::sync::broadcast::Sender<String>,
    /// Automation rules + their runtime state and journal.
    autom: Arc<automations::Engine>,
    tg: Arc<telegram::Store>,
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
    let compact_every = parse_dur(&env("SOLARBOX_COMPACT_EVERY", "1h")).unwrap_or(3_600).max(60);
    let tail = parse_dur(&env("SOLARBOX_COMPACT_TAIL", "2d")).unwrap_or(172_800);
    // Safety net for the router's flash: oldest history goes first past this.
    let max_db: i64 = env("SOLARBOX_MAX_DB_MB", "40").parse::<i64>().unwrap_or(40).max(5) * 1_048_576;

    let conn = db::open(&db_path).expect("open database");
    let db = Arc::new(Mutex::new(conn));
    let tiers = Arc::new(tiers);
    let state = AppState {
        db: db.clone(),
        token: token.clone(),
        tiers: tiers.clone(),
        tail,
        config: Arc::new(config::Store::open(
            PathBuf::from(env("SOLARBOX_CONFIG", "/data/config.json")),
            config::Config::default_with(&env("SOLARBOX_BOARDS", "")),
        )),
        static_dir: PathBuf::from(env("SOLARBOX_STATIC_DIR", "/data/static")),
        admin: std::env::var("SOLARBOX_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty()),
        ota_busy: Arc::new(AtomicBool::new(false)),
        sys: Arc::new(sysinfo::Sampler::new()),
        db_path: db_path.clone(),
        retention_days,
        bus: tokio::sync::broadcast::channel(64).0,
        autom: Arc::new(automations::Engine::open(PathBuf::from(env("SOLARBOX_AUTOMATIONS", "/data/automations.json")))),
        tg: Arc::new(telegram::Store::open(PathBuf::from(env("SOLARBOX_SECRETS", "/data/secrets.json")))),
    };
    telegram::spawn_bot(state.clone());
    {
        // Hold times / staleness need a clock even without new packets.
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                for (rule, clear) in st.autom.tick() {
                    tokio::spawn(automations::execute(st.clone(), rule, clear));
                }
            }
        });
    }
    {
        let sys = state.sys.clone();
        tokio::spawn(async move {
            loop {
                sys.sample();
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
    let control = format!("{} board(s), ui overrides in {}, control {}", state.config.get().boards.len(),
        state.static_dir.display(),
        if state.admin.is_some() { "enabled" } else { "disabled" });

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
                    match db::enforce_cap(&c, max_db) {
                        Ok(n) if n > 0 => eprintln!("[cap] database over limit, dropped {n} oldest points"),
                        Ok(_) => {}
                        Err(e) => eprintln!("[cap] {e}"),
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
        .route("/api/stream", get(stream))
        .route("/api/catalog", get(catalog))
        .route("/api/compact", post(compact_now))
        .route("/api/boards", get(boards::list))
        .route("/api/board/:name/ping", get(boards::ping))
        .route("/api/board/:name/entity", get(boards::entity))
        .route("/api/board/:name/action", post(boards::action))
        .route(
            "/api/board/:name/ota",
            post(boards::flash).layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/api/admin/check", post(boards::admin_check))
        .route("/api/config", get(files::get_config).put(files::put_config))
        .route("/api/files", get(files::list))
        .route("/api/system", get(sysinfo::system))
        .route("/api/automations", get(autom_get).put(autom_put))
        .route("/api/automations/:id/run", post(autom_run))
        .route("/api/telegram", get(tg_get).put(tg_put))
        .route("/api/telegram/test", post(tg_test))
        .route("/api/files/:name", axum::routing::put(files::put).delete(files::delete))
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(|State(st): State<AppState>| async move { files::serve(&st, "index.html") }))
        .route("/*path", get(asset))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind address");
    eprintln!(
        "solarbox on {addr}  db={db_path}  retention={retention_days}d  auth={}  {control}",
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
                Ok(n) => {
                    stored += n;
                    // Nobody listening is fine (send only fails without receivers).
                    let _ = st.bus.send(
                        serde_json::json!({ "device": it.device, "ts": ts, "metrics": it.metrics }).to_string(),
                    );
                    for (rule, clear) in st.autom.on_packet(&it.device, ts, &it.metrics) {
                        tokio::spawn(automations::execute(st.clone(), rule, clear));
                    }
                }
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

// ---- automations ----------------------------------------------------------

fn admin_header(st: &AppState, h: &HeaderMap) -> bool {
    matches!((&st.admin, h.get("x-admin").and_then(|v| v.to_str().ok())), (Some(p), Some(v)) if p == v)
}

async fn autom_get(State(st): State<AppState>, h: HeaderMap) -> Response {
    if !admin_header(&st, &h) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "admin password required" }))).into_response();
    }
    Json(st.autom.snapshot()).into_response()
}

async fn autom_put(State(st): State<AppState>, h: HeaderMap, body: String) -> Response {
    if !admin_header(&st, &h) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "admin password required" }))).into_response();
    }
    match st.autom.save(&body) {
        Ok(n) => Json(serde_json::json!({ "ok": true, "rules": n })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "ok": false, "error": e }))).into_response(),
    }
}

/// Run a rule's actions now (test button), regardless of its condition.
async fn autom_run(State(st): State<AppState>, Path(id): Path<String>, h: HeaderMap) -> Response {
    if !admin_header(&st, &h) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "admin password required" }))).into_response();
    }
    match st.autom.rule(&id) {
        Some(r) => {
            st.autom.log(&r.name, "ручной запуск");
            tokio::spawn(automations::execute(st.clone(), r, false));
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "ok": false, "error": "no such rule" }))).into_response(),
    }
}

// ---- telegram ---------------------------------------------------------------

fn need_admin() -> Response {
    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "admin password required" }))).into_response()
}

/// Whether Telegram is configured (the token itself is never returned).
async fn tg_get(State(st): State<AppState>, h: HeaderMap) -> Response {
    if !admin_header(&st, &h) { return need_admin(); }
    let t = st.tg.get();
    Json(serde_json::json!({ "token_set": t.is_some(), "chat_id": t.map(|t| t.chat_id).filter(|c| !c.is_empty()) })).into_response()
}

#[derive(Deserialize)]
struct TgBody { token: Option<String>, chat_id: Option<String>, #[serde(default)] disable: bool }

/// Set bot token and/or chat id (empty/missing field keeps the stored one);
/// `{"disable": true}` forgets both.
async fn tg_put(State(st): State<AppState>, h: HeaderMap, Json(b): Json<TgBody>) -> Response {
    if !admin_header(&st, &h) { return need_admin(); }
    let cur = st.tg.get();
    let pick = |new: Option<String>, old: Option<String>| new.map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).or(old);
    let token = pick(b.token, cur.as_ref().map(|t| t.token.clone()));
    let chat = pick(b.chat_id, cur.as_ref().map(|t| t.chat_id.clone()).filter(|c| !c.is_empty())).unwrap_or_default();
    let new = match (b.disable, token) {
        (true, _) => None,
        (false, Some(token)) => Some(telegram::Tg { token, chat_id: chat }),
        (false, None) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "ok": false, "error": "нужен токен бота" }))).into_response(),
    };
    match st.tg.set(new) {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "ok": false, "error": e }))).into_response(),
    }
}

async fn tg_test(State(st): State<AppState>, h: HeaderMap) -> Response {
    if !admin_header(&st, &h) { return need_admin(); }
    match telegram::send(&st, "✅ solarbox на связи".into()).await {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "ok": false, "error": e }))).into_response(),
    }
}

// ---- live stream ----------------------------------------------------------

/// Server-sent events: one `data:` line per ingested packet, as it arrives.
async fn stream(
    State(st): State<AppState>,
) -> axum::response::sse::Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::{wrappers::BroadcastStream, StreamExt};
    let s = BroadcastStream::new(st.bus.subscribe())
        .filter_map(|m| m.ok().map(|j| Ok(Event::default().data(j))));
    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
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

// ---- static dashboard (volume overrides, then embedded) -------------------

async fn asset(State(st): State<AppState>, Path(path): Path<String>) -> Response {
    files::serve(&st, &path)
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
