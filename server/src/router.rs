//! The site router's REST API (RouterOS 7, `/rest/...` on its `www` service).
//! Solarbox normally runs in a container on that router, so the router is the
//! container's default gateway; `SOLARBOX_ROUTER` (e.g. `http://192.168.88.1`)
//! overrides. Calls are blocking (ureq) — run them in `spawn_blocking`.

use base64::Engine;
use std::time::Duration;

/// Base URL of the router's REST API, or None when it cannot be found.
pub fn base() -> Option<String> {
    if let Ok(u) = std::env::var("SOLARBOX_ROUTER") {
        let u = u.trim().trim_end_matches('/').to_string();
        return (!u.is_empty()).then_some(u);
    }
    let gw = std::fs::read_to_string("/proc/net/route").ok().and_then(|s| default_gateway(&s))?;
    Some(format!("http://{gw}"))
}

/// Default gateway from `/proc/net/route` (destination 0, gateway in
/// little-endian hex).
fn default_gateway(table: &str) -> Option<std::net::Ipv4Addr> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || f[1] != "00000000" {
            return None;
        }
        let g = u32::from_str_radix(f[2], 16).ok().filter(|g| *g != 0)?;
        Some(std::net::Ipv4Addr::from(g.to_le_bytes()))
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(15)).build()
}

fn basic(user: &str, pw: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pw}")))
}

/// What the router said about a login.
#[derive(Debug, PartialEq)]
pub enum Check {
    Ok,
    Wrong,
    /// No answer, or not a RouterOS REST API (message for the log / the user).
    Unreachable(String),
}

/// Do these credentials log in to the router?
pub fn login(user: &str, pw: &str) -> Check {
    let Some(b) = base() else { return Check::Unreachable("адрес роутера неизвестен".into()) };
    match agent().get(&format!("{b}/rest/system/identity")).set("Authorization", &basic(user, pw)).call() {
        Ok(r) => match r.into_json::<serde_json::Value>() {
            Ok(v) if v.get("name").is_some() => Check::Ok,
            _ => Check::Unreachable("ответ не похож на RouterOS REST".into()),
        },
        Err(ureq::Error::Status(401, _)) => Check::Wrong,
        Err(ureq::Error::Status(code, _)) => Check::Unreachable(format!("роутер ответил HTTP {code}")),
        Err(e) => Check::Unreachable(format!("роутер не отвечает: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gateway() {
        let t = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                 eth0\t000012AC\t00000000\t0001\t0\t0\t0\t0000FFFF\n\
                 eth0\t00000000\t010012AC\t0003\t0\t0\t0\t00000000\n";
        assert_eq!(default_gateway(t), Some("172.18.0.1".parse().unwrap()));
        assert_eq!(default_gateway("Iface\tDestination\tGateway\n"), None);
    }
    #[test]
    fn basic_header() {
        assert_eq!(basic("admin", "pw"), "Basic YWRtaW46cHc=");
    }
}
