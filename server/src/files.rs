//! Live-editable UI. Static files are served from `SOLARBOX_STATIC_DIR` (on
//! the data volume) when present there, otherwise from the copy embedded in
//! the binary — so deleting an override restores the factory UI. Overrides and
//! `config.json` can be replaced over HTTP by the admin; no rebuild, no
//! restart, the browser just reloads.

use crate::{AppState, Assets};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

const MAX_FILE: usize = 2 * 1024 * 1024;

fn admin_ok(st: &AppState, h: &HeaderMap) -> bool {
    matches!((&st.admin, h.get("x-admin").and_then(|v| v.to_str().ok())), (Some(p), Some(v)) if p == v)
}

fn deny(st: &AppState) -> Response {
    let msg = if st.admin.is_none() { "control is disabled: SOLARBOX_ADMIN_PASSWORD is not set" } else { "admin password required" };
    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": msg }))).into_response()
}

fn bad(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "ok": false, "error": msg.into() }))).into_response()
}

/// Flat file names only: letters, digits, `.`, `_`, `-`; no leading dot.
fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with('.')
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Serve a UI file: volume override first, then the embedded copy.
pub fn serve(st: &AppState, path: &str) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream().as_ref().to_string();
    let hdrs = [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache".to_string())];
    if valid_name(path) {
        if let Ok(data) = std::fs::read(st.static_dir.join(path)) {
            return (hdrs, data).into_response();
        }
    }
    match Assets::get(path) {
        Some(f) => (hdrs, f.data.into_owned()).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Which UI files exist and whether each is overridden on the volume.
pub async fn list(State(st): State<AppState>) -> Response {
    let mut names: Vec<String> = Assets::iter().map(|n| n.to_string()).collect();
    let mut over = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&st.static_dir) {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if valid_name(n) {
                    over.push(n.to_string());
                    if !names.contains(&n.to_string()) { names.push(n.to_string()); }
                }
            }
        }
    }
    names.sort();
    let items: Vec<_> = names.iter().map(|n| serde_json::json!({
        "name": n, "override": over.contains(n), "embedded": Assets::get(n).is_some(),
    })).collect();
    Json(serde_json::json!({ "dir": st.static_dir.display().to_string(), "files": items })).into_response()
}

/// Upload / replace a UI file override.
pub async fn put(State(st): State<AppState>, Path(name): Path<String>, h: HeaderMap, body: Bytes) -> Response {
    if !admin_ok(&st, &h) { return deny(&st); }
    if !valid_name(&name) { return bad(StatusCode::BAD_REQUEST, "bad file name"); }
    if body.len() > MAX_FILE { return bad(StatusCode::PAYLOAD_TOO_LARGE, "file too large"); }
    if let Err(e) = std::fs::create_dir_all(&st.static_dir) { return bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()); }
    let tmp = st.static_dir.join(format!(".{name}.tmp"));
    if let Err(e) = std::fs::write(&tmp, &body).and_then(|_| std::fs::rename(&tmp, st.static_dir.join(&name))) {
        return bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    eprintln!("[ui] {name} overridden ({} bytes)", body.len());
    Json(serde_json::json!({ "ok": true, "name": name, "bytes": body.len() })).into_response()
}

/// Remove an override (falls back to the embedded file).
pub async fn delete(State(st): State<AppState>, Path(name): Path<String>, h: HeaderMap) -> Response {
    if !admin_ok(&st, &h) { return deny(&st); }
    if !valid_name(&name) { return bad(StatusCode::BAD_REQUEST, "bad file name"); }
    match std::fs::remove_file(st.static_dir.join(&name)) {
        Ok(_) => { eprintln!("[ui] {name} override removed"); Json(serde_json::json!({ "ok": true })).into_response() }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Json(serde_json::json!({ "ok": true, "note": "no override" })).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Current config (public: it holds no secrets) + last load error, if any.
pub async fn get_config(State(st): State<AppState>) -> Response {
    let cfg = st.config.get();
    let mut v = serde_json::to_value(&*cfg).unwrap_or_default();
    v["load_error"] = serde_json::json!(st.config.error());
    v["admin"] = serde_json::json!(st.admin.is_some());
    Json(v).into_response()
}

/// Replace config.json (validated first).
pub async fn put_config(State(st): State<AppState>, h: HeaderMap, body: String) -> Response {
    if !admin_ok(&st, &h) { return deny(&st); }
    let before = st.config.get();
    match st.config.save(&body) {
        Ok(_) => {
            eprintln!("[config] replaced via API");
            let after: Option<crate::config::Config> = serde_json::from_str(&body).ok();
            let text = match after {
                Some(a) if a.money != before.money => {
                    let cur = &a.money.currency;
                    let lines: Vec<String> = a.money.tariffs.iter().map(|t| {
                        format!("с {} — {} {cur}/кВт·ч{}", t.from, t.price,
                            t.night_price.map(|n| format!(", ночью {n} ({:02}–{:02})", t.night_from, t.night_to)).unwrap_or_default())
                    }).collect();
                    format!("Тариф на электричество:\n{}", if lines.is_empty() { "не задан".into() } else { lines.join("\n") })
                }
                _ => "Конфигурация (config.json) изменена".into(),
            };
            crate::changes::notify(&st, text, crate::changes::By::Dashboard);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

#[cfg(test)]
mod tests {
    use super::valid_name;
    #[test]
    fn names() {
        assert!(valid_name("app.js") && valid_name("style.css") && valid_name("my-page_2.js"));
        for b in ["", "../x", ".env", "a/b", "a b", "x\\y"] { assert!(!valid_name(b), "{b}"); }
    }
}
