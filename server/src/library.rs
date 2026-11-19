//! Board firmware from the release library: the build relay hands out the
//! image for a board's combination (built on demand, reused after), Solarbox
//! checks its signature against the key compiled in here and flashes it over
//! the air. Nothing is flashed that isn't signed.

use crate::{boards, ota, AppState};
use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use std::{sync::atomic::Ordering, time::Duration};

const RELAY: &str = "https://get.solarbox.workers.dev";
const PUBKEY: &str = include_str!("../signing-key.pub");

pub fn verify(data: &[u8], sig: &[u8]) -> bool {
    use base64::Engine;
    let Ok(key) = base64::engine::general_purpose::STANDARD.decode(PUBKEY.trim()) else { return false };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(data, sig).is_ok()
}

/// Library name of a board's image: `<chip>[-pi30]-jk<slots>`.
pub fn target(chip: &str, slots: u64, pi30: bool) -> Option<String> {
    let c = match chip {
        "ESP32-C3" => "c3",
        "ESP32-S3" => "s3",
        "ESP32" => "esp32",
        _ => return None,
    };
    Some(format!("{c}{}-jk{slots}", if pi30 { "-pi30" } else { "" }))
}

fn get(url: &str) -> Result<ureq::Response, String> {
    match ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build().get(url).call() {
        Ok(r) => Ok(r),
        Err(ureq::Error::Status(_, r)) => Ok(r), // the relay explains errors in JSON
        Err(e) => Err(format!("сервер сборок недоступен: {e}")),
    }
}

fn bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut get(url)?.into_reader(), &mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

enum Got {
    Building,
    Image(String, Vec<u8>),
}

/// An image put into `<data>/fw/` by hand (a local build): used before the
/// library, signature checked all the same.
fn local(target: &str, which: &str) -> Option<Result<Got, String>> {
    let dir = std::path::PathBuf::from(std::env::var("SOLARBOX_DATA_DIR").unwrap_or_else(|_| "/data".into())).join("fw");
    let f = dir.join(format!("solarbox-firmware-{target}{}", if which == "ota" { ".ota.bin" } else { ".bin" }));
    let image = std::fs::read(&f).ok()?;
    let sig = std::fs::read(f.with_extension(format!("{}.sig", f.extension()?.to_str()?))).ok()?;
    Some(if verify(&image, &sig) { Ok(Got::Image("своя сборка".into(), image)) } else { Err(format!("{}: подпись не сходится — не прошиваю", f.display())) })
}

/// `which`: "ota" (over the air) or "bin" (factory image, first flash).
fn fetch_file(target: &str, which: &str) -> Result<Got, String> {
    if let Some(r) = local(target, which) {
        return r;
    }
    let v: Value = get(&format!("{RELAY}/v1/firmware/{target}"))?.into_json().map_err(|e| e.to_string())?;
    match v["status"].as_str() {
        Some("building") => Ok(Got::Building),
        Some("ready") => {
            let f = &v["files"][which];
            let (url, sig) = (f["url"].as_str().ok_or("нет образа для обновления")?, f["sig"].as_str().ok_or("нет подписи")?);
            let image = bytes(url)?;
            if !verify(&image, &bytes(sig)?) {
                return Err("подпись прошивки не сходится — не прошиваю".into());
            }
            Ok(Got::Image(v["version"].as_str().unwrap_or("?").to_string(), image))
        }
        Some("failed") => Err("сборка этой прошивки не удалась, попробуйте через полчаса".into()),
        _ => Err(format!("сервер сборок: {}", v["error"].as_str().unwrap_or("?"))),
    }
}

fn fetch(target: &str) -> Result<Got, String> {
    fetch_file(target, "ota")
}

/// The signed factory image (bootloader + partitions + app) for a first
/// flash over USB. Err("building") while the library builds it.
pub fn factory(target: &str) -> Result<(String, Vec<u8>), String> {
    match fetch_file(target, "bin")? {
        Got::Building => Err("building".into()),
        Got::Image(v, i) => Ok((v, i)),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateBody {
    /// Library image to flash; default: from what the board announces.
    #[serde(default)]
    target: Option<String>,
}

/// POST /api/board/:name/update — flash the board with its image from the
/// library. Answers `{status: "building"}` while the image is being built
/// (call again), `{ok, version}` when flashed.
pub async fn update(State(st): State<AppState>, Path(name): Path<String>, body: Option<Json<UpdateBody>>) -> Response {
    let bad = |c: StatusCode, e: String| (c, Json(json!({ "ok": false, "error": e }))).into_response();
    let Some(b) = boards::find(&st, &name) else { return bad(StatusCode::NOT_FOUND, "нет такой платы".into()) };
    let cfg = st.config.get();
    let mac = cfg.board(&name).and_then(|c| c.mac.clone()).unwrap_or_default();
    let seen = st.discovery.boards(&cfg, crate::now()).into_iter().find(|d| d["mac"] == mac.as_str() || d["ip"] == b.host.as_str());
    let target = body.and_then(|Json(x)| x.target).or_else(|| {
        let d = seen.as_ref()?;
        let types: Vec<&str> = d["types"].as_array()?.iter().filter_map(|t| t.as_str()).collect();
        if !types.contains(&"jk-ble") && !types.contains(&"pi30") {
            return None;
        }
        target(d["chip"].as_str()?, d["slots_total"].as_u64()?, types.contains(&"pi30"))
    });
    let Some(target) = target else {
        return bad(StatusCode::BAD_REQUEST, "не знаю, какая прошивка нужна этой плате (старая прошивка не сообщает чип)".into());
    };
    let t = target.clone();
    let got = match tokio::task::spawn_blocking(move || fetch(&t)).await {
        Ok(r) => r,
        Err(e) => Err(e.to_string()),
    };
    let (version, image) = match got {
        Ok(Got::Building) => return Json(json!({ "status": "building", "target": target })).into_response(),
        Ok(Got::Image(v, i)) => (v, i),
        Err(e) => return bad(StatusCode::BAD_GATEWAY, e),
    };
    if st.ota_busy.swap(true, Ordering::SeqCst) {
        return bad(StatusCode::CONFLICT, "уже идёт другая прошивка".into());
    }
    eprintln!("[ota] {name}: {target} {version}, {} bytes → {}:{}", image.len(), b.host, b.ota_port);
    let res = ota::upload(&b.host, b.ota_port, &image).await;
    st.ota_busy.store(false, Ordering::SeqCst);
    match res {
        Ok(msg) => {
            let title = cfg.board(&name).and_then(|c| c.ui.get("title")).and_then(|t| t.as_str()).unwrap_or(&name).to_string();
            crate::changes::notify(&st, format!("{title}: прошивка обновлена до {version} ({target})"), crate::changes::By::Dashboard);
            Json(json!({ "ok": true, "version": version, "target": target, "message": msg })).into_response()
        }
        Err(e) => bad(StatusCode::BAD_GATEWAY, e),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn targets() {
        assert_eq!(super::target("ESP32-S3", 2, false).as_deref(), Some("s3-jk2"));
        assert_eq!(super::target("ESP32-S3", 2, true).as_deref(), Some("s3-pi30-jk2"));
        assert_eq!(super::target("ESP8266", 1, false), None);
    }
    #[test]
    fn rejects_garbage() {
        assert!(!super::verify(b"x", &[0; 64]));
    }
}
