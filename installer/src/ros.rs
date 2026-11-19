//! RouterOS REST (`/rest`, service `www`) with the owner's credentials.

use serde_json::Value;
use std::time::Duration;

pub struct Ros {
    pub host: String,
    pub user: String,
    pub pw: String,
    agent: ureq::Agent,
}

#[derive(Debug)]
pub enum Err {
    /// Wrong login or password.
    Auth,
    /// Nothing answers on the REST port (rebooting, `www` disabled, wrong host).
    Down(String),
    /// The router answered with an error.
    Router(String),
}

impl std::fmt::Display for Err {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Err::Auth => write!(f, "роутер не принял логин или пароль"),
            Err::Down(e) => write!(f, "роутер не отвечает: {e}"),
            Err::Router(e) => write!(f, "роутер ответил ошибкой: {e}"),
        }
    }
}

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

impl Ros {
    pub fn new(host: &str, user: &str, pw: &str) -> Ros {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(60)).build();
        Ros { host: host.into(), user: user.into(), pw: pw.into(), agent }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Err> {
        use base64::Engine;
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.user, self.pw));
        let req = self.agent.request(method, &format!("http://{}/rest{path}", self.host)).set("Authorization", &format!("Basic {auth}"));
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) => Ok(r.into_json::<Value>().unwrap_or(Value::Null)),
            Err(ureq::Error::Status(401, _)) => Err(Err::Auth),
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let m = [s(&v, "detail"), s(&v, "message")].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(": ");
                Err(Err::Router(if m.is_empty() { format!("HTTP {code}") } else { m }))
            }
            Err(e) => Err(Err::Down(e.to_string())),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value, Err> {
        self.call("GET", path, None)
    }
    /// A list endpoint; a single object comes back as a one-element list.
    pub fn list(&self, path: &str) -> Result<Vec<Value>, Err> {
        Ok(match self.get(path)? {
            Value::Array(v) => v,
            Value::Null => vec![],
            v => vec![v],
        })
    }
    pub fn add(&self, path: &str, body: Value) -> Result<Value, Err> {
        self.call("PUT", path, Some(body))
    }
    pub fn post(&self, path: &str, body: Value) -> Result<Value, Err> {
        self.call("POST", path, Some(body))
    }
    /// Add unless an entry matching `query` (e.g. `?name=x`) exists.
    pub fn ensure(&self, path: &str, query: &str, body: Value) -> Result<(), Err> {
        if self.list(&format!("{path}{query}"))?.is_empty() {
            self.add(path, body)?;
        }
        Ok(())
    }

    /// Wait until REST answers again (after a reboot), up to `secs`.
    pub fn wait_up(&self, secs: u64) -> bool {
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        while std::time::Instant::now() < end {
            if self.get("/system/resource").is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        false
    }
}

/// `7.25.1` ≥ `7.25`?
pub fn version_at_least(v: &str, min: (u32, u32)) -> bool {
    let mut p = v.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse::<u32>().ok());
    let (a, b) = (p.next().unwrap_or(0), p.next().unwrap_or(0));
    (a, b) >= min
}

/// Which server build runs on this router; None = no containers there.
pub fn server_arch(arch_name: &str) -> Option<&'static str> {
    match arch_name {
        // armv5 code runs on ARMv7 too, and EN7562CT routers (hAP ax S,
        // hEX refresh) report plain "arm" but only run arm32v5 images.
        "arm" => Some("armv5"),
        "arm64" => Some("arm64"),
        "x86_64" | "x86" => Some("x86_64"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(version_at_least("7.25.1", (7, 25)));
        assert!(version_at_least("7.26", (7, 25)));
        assert!(!version_at_least("7.19.4", (7, 25)));
        assert!(!version_at_least("6.49.8", (7, 25)));
    }
    #[test]
    fn arches() {
        assert_eq!(server_arch("arm"), Some("armv5"));
        assert_eq!(server_arch("mipsbe"), None);
    }
}
