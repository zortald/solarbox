//! HTTPS with a self-signed certificate on the same port as plain HTTP
//! (docs/remote-access.md). The first byte of a connection decides: a TLS
//! handshake goes to the full app, anything else to a small plain app that
//! takes board pushes and redirects browsers to https. The key and certificate
//! are made once (`tls/` on the data volume, 10 years) and never reissued, so
//! the fingerprint a browser accepted stays valid.

use axum::{extract::ConnectInfo, http::{header, HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, Extension, Router};
use base64::Engine;
use hyper_util::{rt::{TokioExecutor, TokioIo}, server::conn::auto, service::TowerToHyperService};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::net::TcpListener;

/// What clients need to know about our certificate.
#[derive(Clone)]
pub struct Info {
    /// SHA-256 of the certificate, `AB:CD:…` as browsers show it.
    pub fingerprint: String,
    /// SHA-256 of the public key, base64 (for `curl --pinnedpubkey sha256//…`).
    pub pin: String,
}

fn sha256(b: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, b).as_ref().to_vec()
}

/// Load `dir/tls/{cert,key}.pem`, creating them on first start.
pub fn load_or_create(dir: &Path) -> Result<(rustls::ServerConfig, Info), String> {
    let d = dir.join("tls");
    let (cp, kp) = (d.join("cert.pem"), d.join("key.pem"));
    if !cp.exists() || !kp.exists() {
        let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
        let mut p = rcgen::CertificateParams::new(vec!["solarbox.local".to_string()]).map_err(|e| e.to_string())?;
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(rcgen::DnType::CommonName, "solarbox");
        let year = (1970 + crate::now() / 31_556_952) as i32;
        p.not_before = rcgen::date_time_ymd(year - 1, 1, 1);
        p.not_after = rcgen::date_time_ymd(year + 10, 1, 1);
        let cert = p.self_signed(&key).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        std::fs::write(&kp, key.serialize_pem()).map_err(|e| format!("{}: {e}", kp.display()))?;
        std::fs::write(&cp, cert.pem()).map_err(|e| format!("{}: {e}", cp.display()))?;
        eprintln!("[tls] new self-signed certificate in {}", d.display());
    }
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cp)
        .map_err(|e| format!("{}: {e}", cp.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", cp.display()))?;
    let first = certs.first().ok_or_else(|| format!("{}: no certificate", cp.display()))?;
    let key_pem = std::fs::read_to_string(&kp).map_err(|e| format!("{}: {e}", kp.display()))?;
    let spki = rcgen::KeyPair::from_pem(&key_pem).map_err(|e| format!("{}: {e}", kp.display()))?.public_key_der();
    let info = Info {
        fingerprint: sha256(first.as_ref()).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"),
        pin: base64::engine::general_purpose::STANDARD.encode(sha256(&spki)),
    };
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| format!("{}: {e}", kp.display()))?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok((cfg, info))
}

/// Plain HTTP for browsers: the same address, but https.
pub async fn to_https(h: HeaderMap, uri: Uri) -> Response {
    let Some(host) = h.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return (StatusCode::BAD_REQUEST, "use https").into_response();
    };
    let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, format!("https://{host}{pq}"))]).into_response()
}

/// Accept connections until `stop` resolves; TLS ones go to `secure`,
/// the rest to `plain`. Both get the peer address as `ConnectInfo`.
pub async fn serve(listener: TcpListener, cfg: rustls::ServerConfig, secure: Router, plain: Router, stop: impl std::future::Future<Output = ()>) {
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    tokio::pin!(stop);
    loop {
        let (tcp, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(x) => x,
                Err(e) => { eprintln!("[http] accept: {e}"); tokio::time::sleep(Duration::from_millis(100)).await; continue; }
            },
            _ = &mut stop => return,
        };
        let (acceptor, secure, plain) = (acceptor.clone(), secure.clone(), plain.clone());
        tokio::spawn(async move {
            let mut b = [0u8; 1];
            // Silent clients (port scanners) are dropped after 10 s.
            match tokio::time::timeout(Duration::from_secs(10), tcp.peek(&mut b)).await {
                Ok(Ok(1)) => {}
                _ => return,
            }
            let r = if b[0] == 0x16 {
                match tokio::time::timeout(Duration::from_secs(15), acceptor.accept(tcp)).await {
                    Ok(Ok(s)) => conn(TokioIo::new(s), secure, peer).await,
                    _ => return, // bad handshake: a browser that refused our certificate, a scanner…
                }
            } else {
                conn(TokioIo::new(tcp), plain, peer).await
            };
            let _ = r;
        });
    }
}

async fn conn<I>(io: I, app: Router, peer: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let svc = TowerToHyperService::new(app.layer(Extension(ConnectInfo(peer))));
    auto::Builder::new(TokioExecutor::new()).serve_connection_with_upgrades(io, svc).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_once_then_reuse() {
        let d = std::env::temp_dir().join(format!("sb-tls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (_, a) = load_or_create(&d).unwrap();
        let (_, b) = load_or_create(&d).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.pin, b.pin);
        assert_eq!(a.fingerprint.len(), 32 * 3 - 1);
    }
}
