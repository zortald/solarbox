//! Remote access (page «Доступ», docs/remote-access.md): login notifications
//! for now.

use crate::AppState;

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
