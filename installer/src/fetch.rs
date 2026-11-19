//! The server image: asked from the build relay (built on demand), downloaded
//! from the release and checked against the signing key compiled in here.

use serde_json::Value;
use std::time::Duration;

pub const RELAY: &str = "https://get.solarbox.workers.dev";
const PUBKEY: &str = include_str!("../../deploy/signing-key.pub");

pub fn verify(data: &[u8], sig: &[u8]) -> bool {
    use base64::Engine;
    let Ok(key) = base64::engine::general_purpose::STANDARD.decode(PUBKEY.trim()) else { return false };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(data, sig).is_ok()
}

fn get(url: &str) -> Result<ureq::Response, ureq::Error> {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(300)).build().get(url).call()
}

fn bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut get(url).map_err(|e| format!("скачивание: {e}"))?.into_reader(), &mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

/// The RouterOS image for `arch`, verified. `progress` gets status lines while
/// it is being built.
pub fn routeros_image(arch: &str, mut progress: impl FnMut(&str)) -> Result<(String, Vec<u8>), String> {
    let started = std::time::Instant::now();
    let mut told: Option<std::time::Instant> = None;
    let files = loop {
        // The relay answers errors as JSON too (4xx/5xx with `error`).
        let v: Value = match get(&format!("{RELAY}/v1/server/{arch}")) {
            Ok(r) => r.into_json().map_err(|e| e.to_string())?,
            Err(ureq::Error::Status(_, r)) => r.into_json().unwrap_or_default(),
            Err(e) => return Err(format!("сервер сборок недоступен: {e}")),
        };
        match v["status"].as_str() {
            Some("ready") => break v,
            Some("building") => {
                if told.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
                    progress(&format!("собирается под {arch} (≈2–5 мин, один раз для всех)… {} с", started.elapsed().as_secs()));
                    told = Some(std::time::Instant::now());
                }
                if started.elapsed() > Duration::from_secs(20 * 60) {
                    return Err("сборка идёт дольше 20 минут — попробуйте позже".into());
                }
                std::thread::sleep(Duration::from_secs(15));
            }
            Some("failed") => return Err(format!("сборка не удалась ({}), попробуйте через полчаса", v["run"].as_str().unwrap_or(""))),
            _ => return Err(format!("сервер сборок ответил: {}", v["error"].as_str().unwrap_or("?"))),
        }
    };
    let version = files["version"].as_str().unwrap_or("?").to_string();
    let f = &files["files"]["routeros"];
    let (url, sig) = (f["url"].as_str().ok_or("нет образа для RouterOS")?, f["sig"].as_str().ok_or("нет подписи")?);
    let data = bytes(url)?;
    if !verify(&data, &bytes(sig)?) {
        return Err("подпись образа не сходится — файл повреждён или подменён, ставить не буду".into());
    }
    Ok((version, data))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_garbage() {
        assert!(!super::verify(b"data", &[0u8; 64]));
    }
}
