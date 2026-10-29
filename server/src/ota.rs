//! ESPHome native OTA client (the protocol `esphome upload` speaks), plaintext
//! and without compression: we advertise no client features, so any ESPHome
//! device answers in its simplest mode. Mirrors esphome/espota2.py
//! `perform_ota` for that path. Devices with an OTA password or encryption are
//! refused with a clear message rather than half-supported.

use md5::{Digest, Md5};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

const MAGIC: [u8; 5] = [0x6C, 0x26, 0xF7, 0x5C, 0x45];
const OK: u8 = 0x00;
const REQUEST_AUTH: u8 = 0x01;
const REQUEST_SHA256_AUTH: u8 = 0x02;
const FEATURE_FLAGS: u8 = 0x48;
const AUTH_OK: u8 = 0x41;
const UPDATE_PREPARE_OK: u8 = 0x42;
const BIN_MD5_OK: u8 = 0x43;
const RECEIVE_OK: u8 = 0x44;
const UPDATE_END_OK: u8 = 0x45;
const CHUNK_OK: u8 = 0x47;
const BLOCK: usize = 8192;

fn device_error(code: u8) -> Option<&'static str> {
    Some(match code {
        0x80 => "invalid magic byte",
        0x81 => "couldn't prepare flash (image too big? try restarting the board)",
        0x82 => "authentication invalid",
        0x83 => "writing flash failed",
        0x84 => "finishing update failed",
        0x85 => "board needs a manual reset after the first USB flash",
        0x86 => "board flashed with wrong flash size",
        0x87 => "image built for a different flash size",
        0x88 => "ESP8266: not enough free space for the image",
        0x89 => "ESP32: OTA partition too small",
        0x8A => "no OTA partition",
        0x8B => "MD5 mismatch",
        0x8D => "firmware signature invalid",
        0x8E => "OTA type not supported",
        0x93 => "downgrade rejected by the board",
        0x94 => "board requires encrypted OTA",
        0xFF => "unknown error from the board",
        _ => return None,
    })
}

async fn read_u8(s: &mut TcpStream, what: &str, wait: Duration) -> Result<u8, String> {
    let mut b = [0u8; 1];
    match timeout(wait, s.read_exact(&mut b)).await {
        Err(_) => Err(format!("timeout waiting for {what}")),
        Ok(Err(e)) => Err(format!("connection lost waiting for {what}: {e}")),
        Ok(Ok(_)) => match device_error(b[0]) {
            Some(msg) => Err(format!("{what}: board says {msg} (0x{:02X})", b[0])),
            None => Ok(b[0]),
        },
    }
}

async fn expect(s: &mut TcpStream, what: &str, want: u8, wait: Duration) -> Result<(), String> {
    let got = read_u8(s, what, wait).await?;
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: unexpected response 0x{got:02X} (want 0x{want:02X})"))
    }
}

async fn send(s: &mut TcpStream, what: &str, data: &[u8]) -> Result<(), String> {
    timeout(Duration::from_secs(30), s.write_all(data))
        .await
        .map_err(|_| format!("timeout sending {what}"))?
        .map_err(|e| format!("sending {what}: {e}"))
}

/// Flash `image` (a firmware .bin as produced by `esphome compile`) to the
/// board's native OTA port. Returns a short human summary on success.
pub async fn upload(host: &str, port: u16, image: &[u8]) -> Result<String, String> {
    if image.len() < 1024 {
        return Err("image is suspiciously small — not a firmware .bin?".into());
    }
    let started = Instant::now();
    let short = Duration::from_secs(15);
    let mut s = timeout(Duration::from_secs(10), TcpStream::connect((host, port)))
        .await
        .map_err(|_| format!("timeout connecting to {host}:{port}"))?
        .map_err(|e| format!("connecting to {host}:{port}: {e}"))?;
    let _ = s.set_nodelay(true);

    send(&mut s, "magic", &MAGIC).await?;
    expect(&mut s, "version", OK, short).await?;
    let version = read_u8(&mut s, "version", short).await?;
    if version != 1 && version != 2 {
        return Err(format!("board speaks OTA version {version}, only 1 and 2 are supported"));
    }

    // No client features: no compression, no SHA256 auth, no extended protocol.
    send(&mut s, "features", &[0]).await?;
    if read_u8(&mut s, "features", short).await? == FEATURE_FLAGS {
        read_u8(&mut s, "feature flags", short).await?;
    }
    match read_u8(&mut s, "auth", short).await? {
        AUTH_OK => {}
        REQUEST_AUTH | REQUEST_SHA256_AUTH => {
            return Err("board has an OTA password — not supported here".into())
        }
        other => return Err(format!("auth: unexpected response 0x{other:02X}")),
    }

    send(&mut s, "size", &(image.len() as u32).to_be_bytes()).await?;
    // Preparing erases flash, which can take a while on an ESP32.
    expect(&mut s, "prepare", UPDATE_PREPARE_OK, Duration::from_secs(60)).await?;
    let md5 = Md5::digest(image);
    let hex: String = md5.iter().map(|b| format!("{b:02x}")).collect();
    send(&mut s, "md5", hex.as_bytes()).await?;
    expect(&mut s, "md5", BIN_MD5_OK, short).await?;

    let _ = s.set_nodelay(false);
    for chunk in image.chunks(BLOCK) {
        send(&mut s, "data", chunk).await?;
        if version >= 2 {
            expect(&mut s, "chunk ack", CHUNK_OK, Duration::from_secs(60)).await?;
        }
    }
    let _ = s.set_nodelay(true);
    let committed = "image was fully sent — the board may already be rebooting with it; \
                     check before retrying";
    expect(&mut s, "receive", RECEIVE_OK, Duration::from_secs(60))
        .await
        .map_err(|e| format!("{e} ({committed})"))?;
    expect(&mut s, "update end", UPDATE_END_OK, Duration::from_secs(60))
        .await
        .map_err(|e| format!("{e} ({committed})"))?;
    let _ = send(&mut s, "end ack", &[OK]).await;

    Ok(format!(
        "{} KB flashed in {:.1} s, board is rebooting",
        image.len() / 1024,
        started.elapsed().as_secs_f32()
    ))
}
