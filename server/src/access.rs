//! Remote access (page «Доступ», docs/remote-access.md): the read-only service
//! user on the router, opening/closing the port forward with a person's router
//! password (used once, never stored), watching the public IP and the daily
//! check that logged-in router users still exist.

use crate::{auth, router, AppState};
use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Duration};

/// Kept in `router.json` on the data volume.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Saved {
    /// Password of the router user `solarbox` (read-only, made by us).
    #[serde(default)]
    service_pw: Option<String>,
    /// External port, chosen when the page is first shown and kept.
    #[serde(default)]
    port: Option<u16>,
    /// Last public IP seen, to notice changes.
    #[serde(default)]
    last_ip: Option<String>,
}

pub struct Access {
    path: PathBuf,
    saved: Mutex<Saved>,
    /// Why the service user could not be made, for the page.
    service_error: Mutex<Option<String>>,
}

impl Access {
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("router.json");
        let saved = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Access { path, saved: Mutex::new(saved), service_error: Mutex::new(None) }
    }

    fn update(&self, f: impl FnOnce(&mut Saved)) {
        let mut s = self.saved.lock().unwrap();
        f(&mut s);
        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, serde_json::to_string_pretty(&*s).unwrap()).and_then(|_| std::fs::rename(&tmp, &self.path)) {
            eprintln!("[access] save {}: {e}", self.path.display());
        }
    }

    fn service(&self) -> Option<router::Cred> {
        let pw = self.saved.lock().unwrap().service_pw.clone()?;
        Some(router::Cred { user: router::SERVICE_USER.into(), pw })
    }

    /// Make (or repair) the service user with a person's credentials.
    /// Blocking.
    fn make_service(&self, admin: &router::Cred) -> Result<(), String> {
        let pw = auth::random_password() + &auth::random_password();
        let r = router::ensure_service_user(admin, &pw).and_then(|_| {
            // Prove it works before relying on it.
            router::call("GET", "/system/identity", &router::Cred { user: router::SERVICE_USER.into(), pw: pw.clone() }, None).map(|_| ())
        });
        match &r {
            Ok(_) => {
                self.update(|s| s.service_pw = Some(pw));
                *self.service_error.lock().unwrap() = None;
                eprintln!("[access] router service user ready");
            }
            Err(e) => {
                *self.service_error.lock().unwrap() = Some(e.clone());
                eprintln!("[access] router service user: {e}");
            }
        }
        r
    }

    /// After a successful router login: make the service user if there is
    /// none or it stopped working. Blocking; the password is not kept.
    pub fn after_login(&self, user: &str, pw: &str) {
        if let Some(svc) = self.service() {
            if router::call("GET", "/system/identity", &svc, None).is_ok() {
                return;
            }
        }
        let _ = self.make_service(&router::Cred { user: user.into(), pw: pw.into() });
    }
}

/// Send a login / access event to Telegram if `notify_access` is on.
/// Never blocks the caller; delivery errors only go to the log.
pub fn notify(st: &AppState, text: String) {
    if !st.tg.get().is_some_and(|t| t.notify_access) {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::telegram::send(&st, text).await {
            eprintln!("[access] telegram: {e}");
        }
    });
}

fn urls(st: &AppState, s: &router::Status, port: u16) -> String {
    let mut u = vec![];
    if let Some(ip) = s.public_ip() {
        u.push(format!("https://{ip}:{port}"));
    }
    if let Some(n) = &s.ddns_name {
        u.push(format!("https://{n}:{port}"));
    }
    format!("{}\nОтпечаток сертификата (SHA-256):\n{}", u.join("\n"), st.tls.fingerprint)
}

/// Public IP every 5 minutes (notify on change while access is open) and,
/// once a day, drop sessions of router users that were removed or disabled.
pub fn spawn_watch(st: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut last_users_check = 0i64;
        loop {
            if let Some(svc) = st.access.service() {
                let s2 = svc.clone();
                match tokio::task::spawn_blocking(move || router::status(&s2)).await {
                    Ok(Ok(s)) => {
                        let ip = s.public_ip();
                        let prev = st.access.saved.lock().unwrap().last_ip.clone();
                        if ip.is_some() && ip != prev {
                            st.access.update(|x| x.last_ip = ip.clone());
                            if let (Some(_), Some(port)) = (prev, s.open_port()) {
                                eprintln!("[access] public IP changed to {}", ip.as_deref().unwrap_or(""));
                                notify(&st, format!("🌐 solarbox: сменился внешний IP\n{}", urls(&st, &s, port)));
                            }
                        }
                    }
                    Ok(Err(e)) => eprintln!("[access] router status: {e}"),
                    Err(e) => eprintln!("[access] {e}"),
                }
                if crate::now() - last_users_check >= 86_400 {
                    last_users_check = crate::now();
                    for user in st.auth.users() {
                        let (s2, u2) = (svc.clone(), user.clone());
                        let state = tokio::task::spawn_blocking(move || router::user_state(&s2, &u2)).await;
                        let why = match state {
                            Ok(Ok(router::UserState::Missing)) => "удалён на роутере",
                            Ok(Ok(router::UserState::Disabled)) => "отключён на роутере",
                            // Active, or the router did not answer: keep the sessions.
                            _ => continue,
                        };
                        let n = st.auth.drop_sessions(Some(&user));
                        eprintln!("[access] dropped {n} session(s) of {user}: {why}");
                        notify(&st, format!("🚪 solarbox: закрыты сессии пользователя {user} — {why}"));
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

// ---- HTTP ------------------------------------------------------------------

/// External port: random in 20000–59999 (fewer scanners than 443; not a defence).
fn random_port() -> u16 {
    let r = auth::random_bytes(2);
    20_000 + (u16::from_le_bytes([r[0], r[1]]) % 40_000)
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "ok": false, "error": msg.into() }))).into_response()
}

/// Everything the «Доступ» page shows.
pub async fn get(State(st): State<AppState>, h: HeaderMap) -> Response {
    let me = st.auth.user(&h).unwrap_or_default();
    let svc = st.access.service();
    let reach = tokio::task::spawn_blocking(router::reachable).await.unwrap_or_else(|e| Err(e.to_string()));
    let status = match (svc, reach) {
        (_, Err(e)) => Err(e),
        (Some(c), _) => tokio::task::spawn_blocking(move || router::status(&c)).await.unwrap_or_else(|e| Err(e.to_string())),
        (None, _) => Err(st.access.service_error.lock().unwrap().clone()
            .map(|e| format!("служебный пользователь не создан: {e}"))
            .unwrap_or_else(|| "solarbox ещё не подключён к роутеру: войдите логином роутера или нажмите «Подключить»".into())),
    };
    // Choose the external port up front, so the manual commands and the
    // button use the same one.
    if st.access.saved.lock().unwrap().port.is_none() {
        let p = random_port();
        st.access.update(|x| x.port = Some(p));
    }
    let saved = st.access.saved.lock().unwrap().clone();
    let port = saved.port.unwrap_or(0);
    let own = router::own_ip().map(|i| i.to_string());
    let sessions: Vec<_> = st.auth.list().iter().map(|s| serde_json::json!({
        "user": s.user, "created": s.created, "seen": s.seen, "ip": s.ip, "browser": auth::browser(&s.ua),
    })).collect();
    let mut v = serde_json::json!({
        "user": me,
        "router": router::base(),
        "own_ip": own,
        "service": st.access.service().is_some(),
        "fingerprint": st.tls.fingerprint,
        "pin": st.tls.pin,
        "port": saved.port,
        "cmd_close": router::close_commands(),
        "sessions": sessions,
    });
    if let (Some(o), true) = (&own, port > 0) {
        v["cmd_open"] = serde_json::json!(router::open_commands(port, o));
    }
    match status {
        Ok(s) => {
            v["status"] = serde_json::json!({
                "wan": s.wan, "wan_list": s.wan_list, "cloud_ip": s.cloud_ip, "ddns_enabled": s.ddns_enabled,
                "ddns_name": s.ddns_name, "public_ip": s.public_ip(), "grey": s.grey(), "open_port": s.open_port(),
                "rules": s.rules.len(),
            });
        }
        Err(e) => {
            if e.starts_with(router::UNREACHABLE) {
                v["cmd_firewall"] = serde_json::json!(router::firewall_command());
            }
            v["status_error"] = serde_json::json!(e);
        }
    }
    Json(v).into_response()
}

#[derive(Deserialize)]
pub struct Confirm {
    /// Router user; defaults to the logged-in one.
    #[serde(default)]
    user: String,
    password: String,
}

fn cred(st: &AppState, h: &HeaderMap, b: Confirm) -> Result<router::Cred, Response> {
    let user = if b.user.trim().is_empty() { st.auth.user(h).unwrap_or_default() } else { b.user.trim().to_string() };
    if user.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "введите логин роутера"));
    }
    if b.password.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "введите пароль роутера"));
    }
    Ok(router::Cred { user, pw: b.password })
}

/// (Re)make the service user.
pub async fn service(State(st): State<AppState>, h: HeaderMap, Json(b): Json<Confirm>) -> Response {
    let c = match cred(&st, &h, b) { Ok(c) => c, Err(r) => return r };
    let st2 = st.clone();
    match tokio::task::spawn_blocking(move || st2.access.make_service(&c)).await {
        Ok(Ok(_)) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Open the port forward (and IP Cloud DDNS).
pub async fn open(State(st): State<AppState>, h: HeaderMap, Json(b): Json<Confirm>) -> Response {
    let c = match cred(&st, &h, b) { Ok(c) => c, Err(r) => return r };
    let saved_port = st.access.saved.lock().unwrap().port;
    let port = saved_port.unwrap_or_else(random_port);
    let st2 = st.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<router::Status, String> {
        let s = router::status(&c)?;
        if s.grey() {
            return Err(format!("у роутера нет белого адреса (WAN: {}) — провайдер или роутер выше по сети делает NAT, проброс отсюда не сработает. Для себя можно пользоваться Back To Home.", s.wan.join(", ")));
        }
        if !s.wan_list {
            return Err("на роутере нет списка интерфейсов WAN — без него правило не к чему привязать".into());
        }
        if st2.access.service().is_none() {
            let _ = st2.access.make_service(&c);
        }
        router::open(&c, port)?;
        router::status(&c)
    }).await;
    match res {
        Ok(Ok(s)) => {
            st.access.update(|x| { x.port = Some(port); x.last_ip = s.public_ip(); });
            eprintln!("[access] opened on port {port}");
            notify(&st, format!("🌐 solarbox: доступ из интернета открыт\n{}", urls(&st, &s, port)));
            Json(serde_json::json!({ "ok": true, "port": port })).into_response()
        }
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Remove the port forward. DDNS and the certificate stay.
pub async fn close(State(st): State<AppState>, h: HeaderMap, Json(b): Json<Confirm>) -> Response {
    let c = match cred(&st, &h, b) { Ok(c) => c, Err(r) => return r };
    match tokio::task::spawn_blocking(move || router::close(&c)).await {
        Ok(Ok(n)) => {
            eprintln!("[access] closed ({n} rule(s) removed)");
            if n > 0 {
                notify(&st, "🔒 solarbox: доступ из интернета закрыт".into());
            }
            Json(serde_json::json!({ "ok": true, "removed": n })).into_response()
        }
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
