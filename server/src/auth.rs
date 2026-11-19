//! Login and sessions (see docs/remote-access.md). One kind of user: whoever
//! logs in has full rights. Credentials are the router's own login, checked
//! against its REST API and never stored, or the emergency password with an
//! empty user name. Sessions are random tokens in a cookie; only their SHA-256
//! is kept (`sessions.json` on the data volume), so a restart logs nobody out.

use crate::{now, router, AppState};
use axum::{
    extract::{ConnectInfo, Json, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::{IpAddr, SocketAddr}, path::PathBuf, sync::Mutex};

const COOKIE: &str = "sb_session";
/// A session lives this long since its last use.
pub const SESSION_TTL: i64 = 30 * 86_400;
/// Failed logins from one IP before the pause starts; the pause starts at
/// FIRST_PAUSE and doubles with every further failure up to MAX_PAUSE.
const FREE_FAILS: u32 = 5;
const FIRST_PAUSE: i64 = 30;
const MAX_PAUSE: i64 = 900;
/// `seen` is written to flash at most this often per session.
const SEEN_SAVE: i64 = 3_600;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Hex SHA-256 of the cookie token.
    hash: String,
    /// Router user name; empty = emergency password.
    pub user: String,
    pub created: i64,
    pub seen: i64,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub ua: String,
    #[serde(skip)]
    saved_seen: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    sessions: Vec<Session>,
}

struct Fail {
    count: u32,
    last: i64,
}

pub struct Auth {
    path: PathBuf,
    sessions: Mutex<Vec<Session>>,
    fails: Mutex<HashMap<IpAddr, Fail>>,
    emergency: String,
}

pub enum LoginErr {
    /// Too many failures: wait this many seconds.
    Pause(i64),
    Wrong,
    /// Router user given but the router cannot be asked.
    Router(String),
}

impl Auth {
    /// Sessions from `dir/sessions.json`; emergency password from
    /// `SOLARBOX_ADMIN_PASSWORD`, else `dir/admin-password` (created and
    /// printed to the log once if missing).
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("sessions.json");
        let mut sessions = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<File>(&s).ok())
            .unwrap_or_default()
            .sessions;
        for s in &mut sessions {
            s.saved_seen = s.seen;
        }
        let emergency = match std::env::var("SOLARBOX_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty()) {
            Some(p) => p,
            None => {
                let f = dir.join("admin-password");
                match std::fs::read_to_string(&f).map(|s| s.trim().to_string()) {
                    Ok(p) if !p.is_empty() => p,
                    _ => {
                        let p = random_password();
                        let _ = std::fs::create_dir_all(dir);
                        match std::fs::write(&f, format!("{p}\n")) {
                            Ok(_) => eprintln!("[auth] emergency password (empty user name, also in {}): {p}", f.display()),
                            Err(e) => eprintln!("[auth] cannot save {}: {e}; emergency password for this run: {p}", f.display()),
                        }
                        p
                    }
                }
            }
        };
        Auth { path, sessions: Mutex::new(sessions), fails: Mutex::new(HashMap::new()), emergency }
    }

    fn save(&self, list: &[Session]) {
        let tmp = self.path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(&File { sessions: list.to_vec() }).unwrap();
        if let Err(e) = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &self.path)) {
            eprintln!("[auth] save sessions: {e}");
        }
    }

    /// The logged-in user for this request ("" = emergency password), if any.
    /// A session cookie or, for scripts, `X-Admin: <emergency password>`.
    pub fn user(&self, h: &HeaderMap) -> Option<String> {
        if let Some(p) = h.get("x-admin").and_then(|v| v.to_str().ok()) {
            return ct_eq(p.as_bytes(), self.emergency.as_bytes()).then(String::new);
        }
        let hash = sha256_hex(cookie(h)?.as_bytes());
        let t = now();
        let mut list = self.sessions.lock().unwrap();
        let s = list.iter_mut().find(|s| s.hash == hash && t - s.seen < SESSION_TTL)?;
        s.seen = t;
        let user = s.user.clone();
        if t - s.saved_seen >= SEEN_SAVE {
            s.saved_seen = t;
            let snapshot = list.clone();
            drop(list);
            self.save(&snapshot);
        }
        Some(user)
    }

    /// Seconds to wait before this IP may try again (0 = may try now).
    fn pause(&self, ip: IpAddr, t: i64) -> i64 {
        let f = self.fails.lock().unwrap();
        match f.get(&ip) {
            Some(x) if x.count >= FREE_FAILS => {
                let wait = (FIRST_PAUSE << (x.count - FREE_FAILS).min(10)).min(MAX_PAUSE);
                (x.last + wait - t).max(0)
            }
            _ => 0,
        }
    }

    /// Count a failure; returns the number of failures in a row.
    fn failed(&self, ip: IpAddr, t: i64) -> u32 {
        let mut f = self.fails.lock().unwrap();
        // Forget IPs that have been quiet for a day.
        f.retain(|_, x| t - x.last < 86_400);
        let x = f.entry(ip).or_insert(Fail { count: 0, last: t });
        x.count += 1;
        x.last = t;
        x.count
    }

    /// Check credentials (blocking: may ask the router) and open a session.
    /// Returns the cookie token.
    pub fn login(&self, user: &str, pw: &str, ip: IpAddr, ua: &str) -> Result<String, (LoginErr, u32)> {
        let t = now();
        let wait = self.pause(ip, t);
        if wait > 0 {
            return Err((LoginErr::Pause(wait), 0));
        }
        let ok = if user.is_empty() {
            Ok(ct_eq(pw.as_bytes(), self.emergency.as_bytes()))
        } else {
            match router::login(user, pw) {
                router::Check::Ok => Ok(true),
                router::Check::Wrong => Ok(false),
                router::Check::Unreachable(e) => {
                    eprintln!("[auth] router check for {user:?}: {e}");
                    Err("роутер не ответил на проверку пароля".to_string())
                }
            }
        };
        match ok {
            Ok(true) => {
                self.fails.lock().unwrap().remove(&ip);
                let token = random_hex(32);
                let s = Session {
                    hash: sha256_hex(token.as_bytes()),
                    user: user.to_string(),
                    created: t,
                    seen: t,
                    ip: ip.to_string(),
                    ua: ua.chars().take(200).collect(),
                    saved_seen: t,
                };
                let mut list = self.sessions.lock().unwrap();
                list.retain(|s| t - s.seen < SESSION_TTL);
                list.push(s);
                let snapshot = list.clone();
                drop(list);
                self.save(&snapshot);
                Ok(token)
            }
            Ok(false) => Err((LoginErr::Wrong, self.failed(ip, t))),
            // Not counted as a failure: the password was not even checked.
            Err(e) => Err((LoginErr::Router(e), 0)),
        }
    }

    pub fn logout(&self, h: &HeaderMap) {
        let Some(c) = cookie(h) else { return };
        let hash = sha256_hex(c.as_bytes());
        let mut list = self.sessions.lock().unwrap();
        list.retain(|s| s.hash != hash);
        let snapshot = list.clone();
        drop(list);
        self.save(&snapshot);
    }

    /// Drop every session (optionally only those of one router user).
    pub fn drop_sessions(&self, user: Option<&str>) -> usize {
        let mut list = self.sessions.lock().unwrap();
        let before = list.len();
        list.retain(|s| user.is_some_and(|u| s.user != u));
        let n = before - list.len();
        let snapshot = list.clone();
        drop(list);
        if n > 0 {
            self.save(&snapshot);
        }
        n
    }

    /// Router users that have live sessions.
    pub fn users(&self) -> Vec<String> {
        let t = now();
        let mut u: Vec<String> = self.sessions.lock().unwrap().iter()
            .filter(|s| !s.user.is_empty() && t - s.seen < SESSION_TTL)
            .map(|s| s.user.clone())
            .collect();
        u.sort();
        u.dedup();
        u
    }

    pub fn list(&self) -> Vec<Session> {
        let t = now();
        self.sessions.lock().unwrap().iter().filter(|s| t - s.seen < SESSION_TTL).cloned().collect()
    }
}

fn cookie(h: &HeaderMap) -> Option<String> {
    h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == COOKIE && !v.is_empty()).then(|| v.to_string())
    })
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    use ring::rand::SecureRandom;
    let mut b = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut b).expect("system random");
    b
}

fn random_hex(n: usize) -> String {
    random_bytes(n).iter().map(|b| format!("{b:02x}")).collect()
}

/// 16 characters without look-alikes (0/O, 1/l/I).
pub fn random_password() -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    random_bytes(16).iter().map(|b| A[*b as usize % A.len()] as char).collect()
}

fn sha256_hex(b: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, b).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// «Chrome, Android» from a User-Agent (good enough for a notification).
pub fn browser(ua: &str) -> String {
    let b = [("Edg/", "Edge"), ("OPR/", "Opera"), ("Firefox/", "Firefox"), ("YaBrowser", "Яндекс"), ("Chrome/", "Chrome"), ("Safari/", "Safari"), ("curl/", "curl")]
        .iter().find(|(k, _)| ua.contains(k)).map(|(_, n)| *n).unwrap_or("браузер");
    let os = [("iPhone", "iPhone"), ("iPad", "iPad"), ("Android", "Android"), ("Windows", "Windows"), ("Mac OS", "macOS"), ("Linux", "Linux")]
        .iter().find(|(k, _)| ua.contains(k)).map(|(_, n)| *n);
    match os { Some(o) => format!("{b}, {o}"), None => b.to_string() }
}

// ---- HTTP ------------------------------------------------------------------

/// Paths reachable without a session.
fn open_path(p: &str) -> bool {
    matches!(p, "/api/ingest" | "/healthz" | "/login.html" | "/api/login" | "/api/logout" | "/style.css" | "/manifest.webmanifest")
        || (p.starts_with("/icon-") && p.ends_with(".png"))
}

/// Gate in front of every route: no session → 401 for the API, the login page
/// for everything else.
pub async fn gate(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let p = req.uri().path();
    if open_path(p) || st.auth.user(req.headers()).is_some() {
        return next.run(req).await;
    }
    if p.starts_with("/api/") {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "нужно войти", "login": true }))).into_response();
    }
    Redirect::to("/login.html").into_response()
}

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    user: String,
    password: String,
}

fn set_cookie(v: &str, max_age: i64) -> (header::HeaderName, String) {
    (header::SET_COOKIE, format!("{COOKIE}={v}; Path=/; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict"))
}

pub async fn login(State(st): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, h: HeaderMap, Json(b): Json<LoginBody>) -> Response {
    let ip = addr.ip().to_canonical();
    let ua = h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let user = b.user.trim().to_string();
    let pw_router = (!user.is_empty()).then(|| b.password.clone());
    let (a, u, pw, ua2) = (st.auth.clone(), user.clone(), b.password, ua.clone());
    let res = match tokio::task::spawn_blocking(move || a.login(&u, &pw, ip, &ua2)).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "ok": false, "error": e.to_string() }))).into_response(),
    };
    match res {
        Ok(token) => {
            let who = if user.is_empty() { "аварийный пароль".to_string() } else { user.clone() };
            eprintln!("[auth] login {who} from {ip}");
            crate::access::notify(&st, format!("🔑 Вход в solarbox: {who}\nIP {ip}, {}", browser(&ua)));
            // A router login is the moment to make the read-only service user.
            if let Some(pw) = pw_router {
                let (acc, u) = (st.access.clone(), user.clone());
                tokio::task::spawn_blocking(move || acc.after_login(&u, &pw));
            }
            ([set_cookie(&token, SESSION_TTL)], Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Err((e, fails)) => {
            let (code, msg) = match e {
                LoginErr::Pause(s) => (StatusCode::TOO_MANY_REQUESTS, format!("слишком много попыток, подождите {s} с")),
                LoginErr::Wrong => (StatusCode::UNAUTHORIZED, "неверный логин или пароль".to_string()),
                LoginErr::Router(e) => (StatusCode::BAD_GATEWAY, format!("{e}. Войдите аварийным паролем (логин пустой)")),
            };
            if fails > 0 {
                eprintln!("[auth] failed login {:?} from {ip} ({fails} in a row)", user);
                if fails == FREE_FAILS || (fails > FREE_FAILS && fails % 20 == 0) {
                    crate::access::notify(&st, format!("⚠️ solarbox: {fails} неудачных попыток входа подряд\nIP {ip}, {}", browser(&ua)));
                }
            }
            (code, Json(serde_json::json!({ "ok": false, "error": msg }))).into_response()
        }
    }
}

pub async fn logout(State(st): State<AppState>, h: HeaderMap) -> Response {
    st.auth.logout(&h);
    ([set_cookie("", 0)], Json(serde_json::json!({ "ok": true }))).into_response()
}

/// «Выйти везде»: every session, this one included.
pub async fn logout_all(State(st): State<AppState>) -> Response {
    let n = st.auth.drop_sessions(None);
    crate::access::notify(&st, format!("🚪 solarbox: закрыты все сессии ({n})"));
    ([set_cookie("", 0)], Json(serde_json::json!({ "ok": true, "dropped": n }))).into_response()
}

/// Who am I + live sessions (for the «Доступ» page).
pub async fn me(State(st): State<AppState>, h: HeaderMap) -> Response {
    let me = st.auth.user(&h).unwrap_or_default();
    let list: Vec<_> = st.auth.list().iter().map(|s| serde_json::json!({
        "user": s.user, "created": s.created, "seen": s.seen, "ip": s.ip, "browser": browser(&s.ua),
    })).collect();
    Json(serde_json::json!({ "user": me, "sessions": list })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-auth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn hdr(k: &str, v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        h
    }

    #[test]
    fn emergency_generated_once_and_kept() {
        let d = tmp("gen");
        std::env::remove_var("SOLARBOX_ADMIN_PASSWORD");
        let a = Auth::open(&d);
        assert_eq!(a.emergency.len(), 16);
        assert_eq!(Auth::open(&d).emergency, a.emergency);
    }

    #[test]
    fn session_roundtrip_and_restart() {
        let d = tmp("sess");
        std::fs::write(d.join("admin-password"), "secret\n").unwrap();
        let a = Auth::open(&d);
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        let tok = a.login("", "secret", ip, "curl/8").ok().unwrap();
        let h = hdr("cookie", &format!("x=1; {COOKIE}={tok}"));
        assert_eq!(a.user(&h), Some(String::new()));
        assert!(!std::fs::read_to_string(d.join("sessions.json")).unwrap().contains(&tok), "token stored in clear");
        // survives a restart
        let b = Auth::open(&d);
        assert_eq!(b.user(&h), Some(String::new()));
        b.logout(&h);
        assert_eq!(b.user(&h), None);
        assert_eq!(b.user(&hdr("cookie", &format!("{COOKIE}=nope"))), None);
    }

    #[test]
    fn x_admin_header() {
        let d = tmp("hdr");
        std::fs::write(d.join("admin-password"), "secret").unwrap();
        let a = Auth::open(&d);
        assert_eq!(a.user(&hdr("x-admin", "secret")), Some(String::new()));
        assert_eq!(a.user(&hdr("x-admin", "secreT")), None);
        assert_eq!(a.user(&HeaderMap::new()), None);
    }

    #[test]
    fn pause_after_five_failures() {
        let d = tmp("rate");
        std::fs::write(d.join("admin-password"), "secret").unwrap();
        let a = Auth::open(&d);
        let ip: IpAddr = "10.0.0.6".parse().unwrap();
        for i in 1..=FREE_FAILS {
            match a.login("", "bad", ip, "") { Err((LoginErr::Wrong, n)) => assert_eq!(n, i), _ => panic!() }
        }
        // the right password is refused too while paused
        assert!(matches!(a.login("", "secret", ip, ""), Err((LoginErr::Pause(_), _))));
        // another IP is not affected
        assert!(a.login("", "secret", "10.0.0.7".parse().unwrap(), "").is_ok());
    }

    #[test]
    fn pause_grows_and_caps() {
        let d = tmp("grow");
        std::fs::write(d.join("admin-password"), "s").unwrap();
        let a = Auth::open(&d);
        let ip: IpAddr = "10.0.0.8".parse().unwrap();
        a.fails.lock().unwrap().insert(ip, Fail { count: 5, last: 1000 });
        assert_eq!(a.pause(ip, 1000), 30);
        assert_eq!(a.pause(ip, 1010), 20);
        a.fails.lock().unwrap().insert(ip, Fail { count: 8, last: 1000 });
        assert_eq!(a.pause(ip, 1000), 240);
        a.fails.lock().unwrap().insert(ip, Fail { count: 40, last: 1000 });
        assert_eq!(a.pause(ip, 1000), MAX_PAUSE);
        assert_eq!(a.pause(ip, 1000 + MAX_PAUSE), 0);
    }

    #[test]
    fn drop_by_user() {
        let d = tmp("drop");
        let a = Auth::open(&d);
        let mk = |u: &str| Session { hash: u.into(), user: u.into(), created: now(), seen: now(), ip: String::new(), ua: String::new(), saved_seen: 0 };
        *a.sessions.lock().unwrap() = vec![mk("ann"), mk("bob"), mk("")];
        assert_eq!(a.users(), vec!["ann", "bob"]);
        assert_eq!(a.drop_sessions(Some("ann")), 1);
        assert_eq!(a.users(), vec!["bob"]);
        assert_eq!(a.drop_sessions(None), 2);
    }

    #[test]
    fn open_paths() {
        for p in ["/api/ingest", "/healthz", "/login.html", "/icon-192.png", "/api/login"] { assert!(open_path(p), "{p}"); }
        for p in ["/", "/index.html", "/api/latest", "/api/stream", "/core.js", "/icon-x.js"] { assert!(!open_path(p), "{p}"); }
    }

    #[test]
    fn browsers() {
        assert_eq!(browser("Mozilla/5.0 (Linux; Android 14) AppleWebKit Chrome/130 Mobile Safari/537"), "Chrome, Android");
        assert_eq!(browser("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Version/18 Mobile Safari/604"), "Safari, iPhone");
        assert_eq!(browser("curl/8.7"), "curl");
    }
}
