//! SSH to the router for what REST cannot do: keep the device-mode update
//! pending (it lives only as long as the session that started it) and upload
//! the image over SFTP.

use russh::client;
use russh::ChannelMsg;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

struct Accept;

// A router on the LAN we just logged into over REST with the owner's password;
// there is no known host key to compare against on a first install.
impl client::Handler for Accept {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub struct Ssh(client::Handle<Accept>);

impl Ssh {
    pub async fn connect(host: &str, port: u16, user: &str, pw: &str) -> Result<Ssh, String> {
        // A router losing power never closes the session: keepalives notice
        // the dead link within ≈15 s.
        let cfg = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(std::time::Duration::from_secs(5)),
            keepalive_max: 3,
            ..Default::default()
        });
        let mut h = client::connect(cfg, (host, port), Accept).await.map_err(|e| format!("SSH к роутеру ({host}:{port}): {e}"))?;
        let ok = h.authenticate_password(user, pw).await.map_err(|e| format!("SSH: {e}"))?;
        if !ok.success() {
            return Err("SSH не принял логин или пароль".into());
        }
        Ok(Ssh(h))
    }

    /// Run a command with a terminal and hand every output line to `line`
    /// until the router closes the session (for device-mode: the reboot).
    pub async fn run_until_closed(&self, cmd: &str, mut line: impl FnMut(&str)) -> Result<(), String> {
        let mut ch = self.0.channel_open_session().await.map_err(|e| e.to_string())?;
        ch.request_pty(false, "dumb", 200, 24, 0, 0, &[]).await.map_err(|e| e.to_string())?;
        ch.exec(true, cmd).await.map_err(|e| e.to_string())?;
        let mut buf = String::new();
        let mut skipped = false;
        while let Some(msg) = ch.wait().await {
            if let ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } = msg {
                buf.push_str(&String::from_utf8_lossy(&data));
                // A router still on its factory password asks to change it
                // before running anything: skip (Ctrl-C), the password stays.
                if !skipped && buf.contains("Change your password") {
                    skipped = true;
                    let _ = ch.data(&b"\x03"[..]).await;
                }
                while let Some(i) = buf.find(['\r', '\n']) {
                    let l: String = buf.drain(..=i).collect();
                    let l = clean(&l);
                    // After the factory-password prompt RouterOS keeps the
                    // session open at its CLI prompt: the command is over.
                    if l.starts_with('[') && l.contains("] >") {
                        let _ = ch.close().await;
                        return Ok(());
                    }
                    if !l.is_empty() {
                        line(&l);
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn upload(&self, name: &str, data: &[u8]) -> Result<(), String> {
        let ch = self.0.channel_open_session().await.map_err(|e| e.to_string())?;
        ch.request_subsystem(true, "sftp").await.map_err(|e| e.to_string())?;
        let sftp = russh_sftp::client::SftpSession::new(ch.into_stream()).await.map_err(|e| format!("SFTP: {e}"))?;
        let mut f = sftp.create(name).await.map_err(|e| format!("SFTP: {e}"))?;
        f.write_all(data).await.map_err(|e| format!("SFTP: {e}"))?;
        f.shutdown().await.map_err(|e| format!("SFTP: {e}"))?;
        Ok(())
    }
}

/// Drop terminal escape sequences and the RouterOS status prompt.
fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !c.is_control() {
            out.push(c);
        }
    }
    out.replace("-- [Q quit|D dump|C-z pause]", "").trim().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn clean() {
        assert_eq!(super::clean("\x1b[K-- [Q quit|D dump|C-z pause]  update: turn off power in 4m58s to activate changes\r"), "update: turn off power in 4m58s to activate changes");
    }
}
