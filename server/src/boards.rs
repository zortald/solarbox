//! Board control. Telemetry stays push-only; these endpoints reach a board's
//! ESPHome web API (or its native OTA port) only when someone acts in the UI.
//!
//! Boards and the rules for what may be read/done on them live in
//! `config.json` (see config.rs), hot-reloaded. The board name matches the
//! `device` it pushes as. Reads are open but limited to the configured state
//! domains (never `text`, which may hold Wi-Fi credentials); anything that
//! changes state needs `X-Admin: <SOLARBOX_ADMIN_PASSWORD>` and a matching rule.

use crate::{db, ota, AppState};
use axum::{
    body::Bytes,
    extract::{Json, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

#[derive(Clone, Debug)]
pub struct Board {
    pub name: String,
    pub host: String,
    pub web_port: u16,
    pub ota_port: u16,
}

pub fn parse_boards(s: &str) -> Vec<Board> {
    s.split(',')
        .filter_map(|part| {
            let (name, rest) = part.trim().split_once('=')?;
            let mut it = rest.trim().split(':');
            let host = it.next()?.to_string();
            let web_port = it.next().unwrap_or("80").parse().ok()?;
            let ota_port = it.next().unwrap_or("8266").parse().ok()?;
            Some(Board { name: name.trim().to_string(), host, web_port, ota_port })
        })
        .collect()
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "ok": false, "error": msg.into() }))).into_response()
}

fn find(st: &AppState, name: &str) -> Option<Board> {
    st.config.get().board(name).map(|b| Board {
        name: b.name.clone(),
        host: b.host.clone(),
        web_port: b.web_port,
        ota_port: b.ota_port,
    })
}

fn is_admin(st: &AppState, headers: &HeaderMap) -> bool {
    match &st.admin {
        Some(pw) => headers
            .get("x-admin")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == pw)
            .unwrap_or(false),
        None => false,
    }
}

fn admin_denied(st: &AppState) -> Response {
    if st.admin.is_none() {
        err(StatusCode::FORBIDDEN, "control is disabled: SOLARBOX_ADMIN_PASSWORD is not set")
    } else {
        err(StatusCode::UNAUTHORIZED, "admin password required")
    }
}

/// Percent-encode one path segment / query value.
pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Minimal HTTP/1.1 request to a board (plain HTTP on the LAN, Connection:
/// close). Returns status and body.
async fn http(b: &Board, method: &str, path: &str) -> Result<(u16, String), String> {
    let mut s = timeout(Duration::from_secs(4), TcpStream::connect((b.host.as_str(), b.web_port)))
        .await
        .map_err(|_| "board unreachable (connect timeout)".to_string())?
        .map_err(|e| format!("board unreachable: {e}"))?;
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        b.host
    );
    s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    // Some boards (ESP-IDF httpd) ignore "Connection: close", so stop as soon
    // as the response is complete instead of waiting for EOF.
    let mut buf = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut chunk = [0u8; 2048];
    loop {
        if response_complete(&buf) {
            break;
        }
        match tokio::time::timeout_at(deadline, s.read(&mut chunk)).await {
            Err(_) if buf.is_empty() => return Err("board did not answer in time".into()),
            Err(_) => break,
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
            Ok(Err(e)) => return Err(e.to_string()),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("malformed response from board")?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    Ok((status, if chunked { dechunk(body) } else { body.to_string() }))
}

/// True once `buf` holds a full HTTP response (by Content-Length, or the final
/// zero-size chunk). Without either we can only rely on EOF.
fn response_complete(buf: &[u8]) -> bool {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let body = &buf[end + 4..];
    for line in head.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            return v.trim().parse::<usize>().map(|n| body.len() >= n).unwrap_or(false);
        }
        if line.starts_with("transfer-encoding:") && line.contains("chunked") {
            return body.ends_with(b"0\r\n\r\n");
        }
    }
    false
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        s = rest[n..].trim_start_matches("\r\n");
    }
    out
}

// ---- handlers ---------------------------------------------------------------

/// Configured boards + when each last pushed (no network).
pub async fn list(State(st): State<AppState>) -> Response {
    let seen = {
        let c = st.db.lock().unwrap();
        db::last_push(&c).unwrap_or_default()
    };
    let cfg = st.config.get();
    let items: Vec<_> = cfg
        .boards
        .iter()
        .map(|b| {
            let mut o = serde_json::to_value(b).unwrap_or_default();
            o["last_push"] = serde_json::json!(seen.iter().find(|(d, _)| d == &b.name).map(|(_, t)| *t));
            o
        })
        .collect();
    Json(serde_json::json!({ "boards": items, "admin": st.admin.is_some() })).into_response()
}

/// Reachability + latency of the board's web server.
pub async fn ping(State(st): State<AppState>, Path(name): Path<String>) -> Response {
    let Some(b) = find(&st, &name) else { return err(StatusCode::NOT_FOUND, "no such board") };
    let b = &b;
    let t = Instant::now();
    match http(b, "GET", "/").await {
        Ok((code, _)) => Json(serde_json::json!({
            "ok": code < 500, "status": code, "ms": t.elapsed().as_millis() as u64
        }))
        .into_response(),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })).into_response(),
    }
}

#[derive(Deserialize)]
pub struct EntityQ {
    /// "domain/entity name", e.g. "text_sensor/inverter raw qpiri"
    path: String,
}

/// Read one entity's current state from the board (state domains only).
pub async fn entity(State(st): State<AppState>, Path(name): Path<String>, Query(q): Query<EntityQ>) -> Response {
    let Some(b) = find(&st, &name) else { return err(StatusCode::NOT_FOUND, "no such board") };
    let b = &b;
    let Some((domain, ent)) = q.path.split_once('/') else {
        return err(StatusCode::BAD_REQUEST, "path must be domain/name");
    };
    if !st.config.get().read_domains.iter().any(|d| d == domain) {
        return err(StatusCode::FORBIDDEN, format!("reading {domain} is not allowed"));
    }
    match http(b, "GET", &format!("/{domain}/{}", pct(ent))).await {
        Ok((200, body)) => ([("content-type", "application/json")], body).into_response(),
        Ok((code, _)) => err(StatusCode::BAD_GATEWAY, format!("board answered HTTP {code}")),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

#[derive(Deserialize)]
pub struct ActionBody {
    /// "domain/entity name"
    path: String,
    /// turn_on | turn_off | toggle | press | set
    action: String,
    value: Option<String>,
}

/// Change something on a board (admin only, whitelisted per domain).
pub async fn action(
    State(st): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(a): Json<ActionBody>,
) -> Response {
    if !is_admin(&st, &headers) {
        return admin_denied(&st);
    }
    let Some(b) = find(&st, &name) else { return err(StatusCode::NOT_FOUND, "no such board") };
    let b = &b;
    let Some((domain, ent)) = a.path.split_once('/') else {
        return err(StatusCode::BAD_REQUEST, "path must be domain/name");
    };
    if !st.config.get().allows(&name, domain, ent, &a.action, a.value.as_deref()) {
        return err(StatusCode::FORBIDDEN, format!("{} on {} is not allowed", a.action, a.path));
    }
    let mut path = format!("/{domain}/{}/{}", pct(ent), a.action);
    if let Some(v) = &a.value {
        path.push_str(&format!("?value={}", pct(v)));
    }
    eprintln!("[control] {name}: POST {path}");
    match http(b, "POST", &path).await {
        Ok((code, _)) if code < 300 => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok((code, body)) => err(StatusCode::BAD_GATEWAY, format!("board answered HTTP {code}: {}", body.trim())),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// Flash a firmware .bin (request body) to the board over native OTA.
pub async fn flash(
    State(st): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    image: Bytes,
) -> Response {
    if !is_admin(&st, &headers) {
        return admin_denied(&st);
    }
    let Some(b) = find(&st, &name) else { return err(StatusCode::NOT_FOUND, "no such board") };
    if st.ota_busy.swap(true, Ordering::SeqCst) {
        return err(StatusCode::CONFLICT, "another firmware upload is in progress");
    }
    eprintln!("[ota] {name}: {} bytes → {}:{}", image.len(), b.host, b.ota_port);
    let res = ota::upload(&b.host, b.ota_port, &image).await;
    st.ota_busy.store(false, Ordering::SeqCst);
    match res {
        Ok(msg) => {
            eprintln!("[ota] {name}: {msg}");
            Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            eprintln!("[ota] {name}: FAILED {e}");
            err(StatusCode::BAD_GATEWAY, e)
        }
    }
}

/// Lets the UI verify a password before showing control actions.
pub async fn admin_check(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if is_admin(&st, &headers) {
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        admin_denied(&st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let b = parse_boards("inverter=192.168.88.10:80:8266, bms=192.168.88.11:80:3232");
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].name, "bms");
        assert_eq!(b[1].ota_port, 3232);
        assert_eq!(pct("inverter raw qpiri"), "inverter%20raw%20qpiri");
    }

    #[test]
    fn complete() {
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel"));
        assert!(response_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"));
        assert!(response_complete("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nЖЖ".as_bytes()));
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n"));
        assert!(response_complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"));
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\n\r\nno length"));
    }

    #[test]
    fn chunks() {
        assert_eq!(dechunk("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"), "hello world");
    }
}
