//! Finding MikroTik routers on the LAN: MNDP, the neighbour discovery Winbox
//! uses. A zero request to the broadcast address on UDP 5678; routers answer
//! to the broadcast address on that port, so we listen there.

use std::{net::UdpSocket, time::{Duration, Instant}};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Router {
    pub ip: String,
    pub identity: String,
    pub board: String,
    pub version: String,
}

/// TLVs after the 4-byte header: type u16, length u16, value.
pub fn parse(pkt: &[u8], from: &str) -> Option<Router> {
    if pkt.len() <= 4 {
        return None; // our own request, or another scanner's
    }
    let mut r = Router { ip: from.to_string(), ..Default::default() };
    let mut i = 4;
    while i + 4 <= pkt.len() {
        let t = u16::from_be_bytes([pkt[i], pkt[i + 1]]);
        let l = u16::from_be_bytes([pkt[i + 2], pkt[i + 3]]) as usize;
        let v = pkt.get(i + 4..i + 4 + l)?;
        let s = || String::from_utf8_lossy(v).trim().to_string();
        match t {
            5 => r.identity = s(),
            7 => r.version = s().split_whitespace().next().unwrap_or("").to_string(),
            12 => r.board = s(),
            17 if l == 4 => r.ip = format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]),
            _ => {}
        }
        i += 4 + l;
    }
    Some(r)
}

/// Routers that answer within `wait`, sorted by address.
pub fn scan(wait: Duration) -> Vec<Router> {
    let sock = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).ok();
    let sock: Option<UdpSocket> = sock.and_then(|s| {
        let _ = s.set_reuse_address(true);
        #[cfg(unix)]
        let _ = s.set_reuse_port(true);
        let addr: std::net::SocketAddr = "0.0.0.0:5678".parse().unwrap();
        s.bind(&addr.into()).ok().map(|_| s.into())
    });
    let Some(sock) = sock.or_else(|| UdpSocket::bind("0.0.0.0:0").ok()) else { return vec![] };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = sock.send_to(&[0, 0, 0, 0], "255.255.255.255:5678");
    let mut out: Vec<Router> = Vec::new();
    let (end, mut buf) = (Instant::now() + wait, [0u8; 2048]);
    while Instant::now() < end {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some(r) = parse(&buf[..n], &from.ip().to_string()) {
                if !out.iter().any(|x| x.ip == r.ip) {
                    out.push(r);
                }
            }
        }
    }
    out.sort_by_key(|r| r.ip.split('.').map(|x| x.parse::<u32>().unwrap_or(0)).fold(0, |a, b| a * 256 + b));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_an_answer() {
        let mut p = vec![0, 0, 0, 1];
        for (t, v) in [(5u16, b"hAP".to_vec()), (7, b"7.25 2026-10-22".to_vec()), (12, b"E62iUGS-2axD5axT".to_vec()), (17, vec![10, 10, 0, 20])] {
            p.extend_from_slice(&t.to_be_bytes());
            p.extend_from_slice(&(v.len() as u16).to_be_bytes());
            p.extend_from_slice(&v);
        }
        let r = super::parse(&p, "10.10.0.99").unwrap();
        assert_eq!((r.ip.as_str(), r.identity.as_str(), r.version.as_str(), r.board.as_str()), ("10.10.0.20", "hAP", "7.25", "E62iUGS-2axD5axT"));
        assert!(super::parse(&[0, 0, 0, 0], "x").is_none());
    }
}
