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

// ---- REST calls with given credentials ----------------------------------------

/// Router credentials: the service user's, or a person's for one change.
#[derive(Clone)]
pub struct Cred {
    pub user: String,
    pub pw: String,
}

/// One REST call; error text is what RouterOS said (`detail`/`message`).
pub fn call(method: &str, path: &str, c: &Cred, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let b = base().ok_or("адрес роутера неизвестен")?;
    let req = agent().request(method, &format!("{b}/rest{path}")).set("Authorization", &basic(&c.user, &c.pw));
    let r = match body {
        Some(v) => req.send_json(v),
        None => req.call(),
    };
    match r {
        Ok(r) => {
            let s = r.into_string().map_err(|e| e.to_string())?;
            if s.trim().is_empty() { Ok(serde_json::Value::Null) } else { serde_json::from_str(&s).map_err(|e| e.to_string()) }
        }
        Err(ureq::Error::Status(401, _)) => Err("роутер не принял логин или пароль".into()),
        Err(ureq::Error::Status(code, r)) => {
            let v: serde_json::Value = r.into_json().unwrap_or_default();
            let msg = [v.get("detail"), v.get("message")].into_iter().flatten().filter_map(|x| x.as_str()).next().unwrap_or("").to_string();
            Err(format!("роутер: HTTP {code} {msg}").trim().to_string())
        }
        Err(e) => Err(format!("роутер не отвечает: {e}")),
    }
}

fn list(path: &str, c: &Cred) -> Result<Vec<serde_json::Value>, String> {
    match call("GET", path, c, None)? {
        serde_json::Value::Array(v) => Ok(v),
        v => Err(format!("неожиданный ответ роутера: {v}")),
    }
}

fn s<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// This container's address as the router sees it (source address towards
/// the gateway).
pub fn own_ip() -> Option<std::net::IpAddr> {
    let b = base()?;
    let host = b.split("://").nth(1)?.split(['/', ':']).next()?.to_string();
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect((host.as_str(), 80)).ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

pub const SERVICE_USER: &str = "solarbox";
const SERVICE_GROUP: &str = "solarbox-read";
/// NAT rules made by «Доступ» carry this comment (the LAN rule from the
/// install guide is plain "solarbox" and is never touched).
pub const NAT_COMMENT: &str = "solarbox-access";

/// Create (or reset) the read-only service user with `pw`, using a person's
/// credentials. Allowed only from this container's address.
pub fn ensure_service_user(admin: &Cred, pw: &str) -> Result<(), String> {
    if list(&format!("/user/group?name={SERVICE_GROUP}"), admin)?.is_empty() {
        call("PUT", "/user/group", admin, Some(serde_json::json!({
            "name": SERVICE_GROUP, "policy": "read,rest-api", "comment": "solarbox: только чтение",
        })))?;
    }
    let mut u = serde_json::json!({ "group": SERVICE_GROUP, "password": pw, "disabled": "false" });
    if let Some(ip) = own_ip() {
        u["address"] = serde_json::json!(format!("{ip}/32"));
    }
    match list(&format!("/user?name={SERVICE_USER}"), admin)?.first() {
        Some(x) => { call("PATCH", &format!("/user/{}", s(x, ".id")), admin, Some(u))?; }
        None => {
            u["name"] = serde_json::json!(SERVICE_USER);
            u["comment"] = serde_json::json!("solarbox: служебный, только чтение");
            call("PUT", "/user", admin, Some(u))?;
        }
    }
    Ok(())
}

/// A router user as seen by the daily session check.
#[derive(Debug, PartialEq)]
pub enum UserState {
    Active,
    Disabled,
    Missing,
}

pub fn user_state(svc: &Cred, name: &str) -> Result<UserState, String> {
    let q: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    Ok(match list(&format!("/user?name={q}"), svc)?.first() {
        None => UserState::Missing,
        Some(u) if s(u, "disabled") == "true" => UserState::Disabled,
        Some(_) => UserState::Active,
    })
}

/// Private, CGNAT, link-local or loopback: not reachable from the internet.
pub fn is_grey(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || (o[0] == 100 && (64..128).contains(&o[1]))
}

/// What the router says about reachability from the internet.
#[derive(Debug, Default, serde::Serialize)]
pub struct Status {
    /// Addresses on interfaces of the WAN list.
    pub wan: Vec<String>,
    /// The address the internet sees (IP Cloud), when known.
    pub cloud_ip: Option<String>,
    pub ddns_enabled: bool,
    pub ddns_name: Option<String>,
    pub wan_list: bool,
    /// Our forwarding rules: (id, external port, disabled).
    pub rules: Vec<(String, u16, bool)>,
}

impl Status {
    /// Public address reachable from outside, if any.
    pub fn public_ip(&self) -> Option<String> {
        let public = |a: &str| a.split('/').next()?.parse::<std::net::Ipv4Addr>().ok().filter(|ip| !is_grey(*ip));
        if let Some(w) = self.wan.iter().find_map(|a| public(a)) {
            return Some(w.to_string());
        }
        // No address on the WAN list at all (e.g. a PPPoE interface left out
        // of it): fall back to what IP Cloud sees.
        if self.wan.is_empty() {
            return self.cloud_ip.as_deref().and_then(public).map(|ip| ip.to_string());
        }
        None
    }
    /// The WAN has only grey addresses (provider NAT / another router in front).
    pub fn grey(&self) -> bool {
        self.wan_list && !self.wan.is_empty() && self.public_ip().is_none()
    }
    pub fn open_port(&self) -> Option<u16> {
        self.rules.iter().find(|r| !r.2).map(|r| r.1)
    }
}

pub fn status(c: &Cred) -> Result<Status, String> {
    let mut st = Status::default();
    let members = list("/interface/list/member?list=WAN", c)?;
    st.wan_list = !list("/interface/list?name=WAN", c)?.is_empty();
    let ifs: Vec<&str> = members.iter().map(|m| s(m, "interface")).collect();
    for a in list("/ip/address", c)? {
        if ifs.contains(&s(&a, "interface")) && s(&a, "disabled") != "true" {
            st.wan.push(s(&a, "address").to_string());
        }
    }
    let cloud = call("GET", "/ip/cloud", c, None)?;
    st.ddns_enabled = s(&cloud, "ddns-enabled") == "yes";
    st.cloud_ip = Some(s(&cloud, "public-address").to_string()).filter(|x| !x.is_empty());
    st.ddns_name = Some(s(&cloud, "dns-name").to_string()).filter(|x| !x.is_empty());
    for r in list(&format!("/ip/firewall/nat?comment={NAT_COMMENT}"), c)? {
        if let Ok(p) = s(&r, "dst-port").parse() {
            st.rules.push((s(&r, ".id").to_string(), p, s(&r, "disabled") == "true"));
        }
    }
    Ok(st)
}

/// RouterOS commands that open access (also shown for pasting by hand).
pub fn open_commands(port: u16, to: &str) -> String {
    format!("/ip/cloud/set ddns-enabled=yes\n/ip/firewall/nat/add chain=dstnat in-interface-list=WAN protocol=tcp dst-port={port} action=dst-nat to-addresses={to} to-ports=8080 comment={NAT_COMMENT}")
}

pub fn close_commands() -> String {
    format!("/ip/firewall/nat/remove [find comment={NAT_COMMENT}]")
}

/// Open access on `port` with a person's credentials.
pub fn open(admin: &Cred, port: u16) -> Result<(), String> {
    let to = own_ip().ok_or("не удалось узнать адрес контейнера")?.to_string();
    if list("/interface/list?name=WAN", admin)?.is_empty() {
        return Err("на роутере нет списка интерфейсов WAN".into());
    }
    call("POST", "/ip/cloud/set", admin, Some(serde_json::json!({ "ddns-enabled": "yes" })))?;
    for r in list(&format!("/ip/firewall/nat?comment={NAT_COMMENT}"), admin)? {
        call("DELETE", &format!("/ip/firewall/nat/{}", s(&r, ".id")), admin, None)?;
    }
    call("PUT", "/ip/firewall/nat", admin, Some(serde_json::json!({
        "chain": "dstnat", "action": "dst-nat", "in-interface-list": "WAN", "protocol": "tcp",
        "dst-port": port.to_string(), "to-addresses": to, "to-ports": "8080", "comment": NAT_COMMENT,
    })))?;
    Ok(())
}

/// Remove our forwarding rules with a person's credentials; returns how many.
pub fn close(admin: &Cred) -> Result<usize, String> {
    let rules = list(&format!("/ip/firewall/nat?comment={NAT_COMMENT}"), admin)?;
    for r in &rules {
        call("DELETE", &format!("/ip/firewall/nat/{}", s(r, ".id")), admin, None)?;
    }
    Ok(rules.len())
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
    fn grey_addresses() {
        for a in ["10.1.2.3", "172.20.0.1", "192.168.88.1", "100.64.0.1", "100.127.255.1", "169.254.1.1"] { assert!(is_grey(a.parse().unwrap()), "{a}"); }
        for a in ["100.128.0.1", "93.184.216.34", "172.32.0.1"] { assert!(!is_grey(a.parse().unwrap()), "{a}"); }
    }
    #[test]
    fn status_public() {
        let mut s = Status { wan_list: true, wan: vec!["100.70.1.2/10".into()], ..Default::default() };
        assert!(s.grey() && s.public_ip().is_none());
        s.wan.push("93.184.216.34/24".into());
        assert_eq!(s.public_ip().as_deref(), Some("93.184.216.34"));
        assert!(!s.grey());
        let c = Status { wan_list: true, cloud_ip: Some("93.184.216.34".into()), ..Default::default() };
        assert_eq!(c.public_ip().as_deref(), Some("93.184.216.34"));
        s.rules = vec![("*1".into(), 40000, true), ("*2".into(), 41000, false)];
        assert_eq!(s.open_port(), Some(41000));
    }
    #[test]
    fn basic_header() {
        assert_eq!(basic("admin", "pw"), "Basic YWRtaW46cHc=");
    }
}
