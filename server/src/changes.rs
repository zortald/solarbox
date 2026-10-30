//! Every change of settings is mirrored to Telegram (optional, on by default:
//! `notify_changes` in the Telegram settings). This module turns raw board
//! actions — ESPHome entity paths and PI30 command strings — into plain text.

use crate::AppState;

/// Who made the change.
pub enum By<'a> {
    Dashboard,
    Automation(&'a str),
}

/// Send `text` (plus who did it) to Telegram if change notifications are on.
/// Never blocks the caller; delivery errors only go to the log.
pub fn notify(st: &AppState, text: String, by: By) {
    if !st.tg.get().is_some_and(|t| t.notify_changes) {
        return;
    }
    let who = match by {
        By::Dashboard => "👤 из дашборда".to_string(),
        By::Automation(name) => format!("🤖 автоматизация «{name}»"),
    };
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::telegram::send(&st, format!("⚙️ {text}\n{who}")).await {
            eprintln!("[changes] telegram: {e}");
        }
    });
}

/// "Инвертор: Приоритет заряда → Только солнце" for a successful board action.
pub fn describe_action(st: &AppState, board: &str, domain: &str, ent: &str, action: &str, value: Option<&str>) -> String {
    let cfg = st.config.get();
    let title = cfg
        .board(board)
        .and_then(|b| b.ui.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or(board)
        .to_string();
    let ent = ent.strip_prefix(&format!("{board} ")).unwrap_or(ent);
    let what = match (domain, action, value) {
        ("text", "set", Some(v)) if ent == "command" => pi30(v),
        ("switch", "turn_on", _) if ent.starts_with("output_source_priority_") => {
            format!("Приоритет выхода → {}", match &ent["output_source_priority_".len()..] {
                "utility" => "Сеть",
                "solar" => "Солнце",
                "battery" => "Батарея (SBU)",
                x => x,
            })
        }
        ("switch", a, _) => format!("{} → {}", entity_name(ent), match a {
            "turn_on" => "вкл",
            "turn_off" => "выкл",
            _ => "переключено",
        }),
        ("button", _, _) => format!("нажата кнопка «{}»", entity_name(ent)),
        (_, "set", Some(v)) => format!("{} → {v}", entity_name(ent)),
        (_, a, v) => format!("{} {a}{}", entity_name(ent), v.map(|v| format!(" {v}")).unwrap_or_default()),
    };
    format!("{title}: {what}")
}

fn entity_name(ent: &str) -> String {
    match ent {
        "pv_power_balance" => "Баланс мощности PV".into(),
        e => e.replace('_', " "),
    }
}

/// PI30 setting commands in words (the ones the inverter page sends).
pub fn pi30(cmd: &str) -> String {
    let num = |p: &str| cmd.strip_prefix(p).map(|s| s.trim_start_matches('0').to_string());
    let pick = |p: &str, names: &[&str]| -> Option<String> {
        let i: usize = cmd.strip_prefix(p)?.parse().ok()?;
        Some(names.get(i).map(|s| s.to_string()).unwrap_or_else(|| i.to_string()))
    };
    let flag = |c: char| match c {
        'a' => "Звуковой сигнал",
        'b' => "Байпас при перегрузке",
        'j' => "Энергосбережение",
        'k' => "Возврат экрана через 1 мин",
        'u' => "Авторестарт после перегрузки",
        'v' => "Авторестарт после перегрева",
        'x' => "Подсветка экрана",
        'y' => "Сигнал при пропадании сети",
        'z' => "Запись кодов ошибок",
        _ => "?",
    };
    let v = |p: &str| cmd.strip_prefix(p).map(String::from);
    let r = if let Some(s) = pick("POP", &["Сеть", "Солнце", "Батарея (SBU)"]) {
        format!("Приоритет выхода → {s}")
    } else if let Some(s) = pick("PCP", &["Сначала сеть", "Сначала солнце", "Солнце + сеть", "Только солнце"]) {
        format!("Приоритет заряда → {s}")
    } else if let Some(s) = pick("PGR", &["Бытовой (90–280 В)", "ИБП (170–280 В)"]) {
        format!("Диапазон входа → {s}")
    } else if let Some(s) = pick("PBT", &["AGM", "Заливная", "Пользовательская", "Pylontech", "Shinheung", "Weco", "Soltaro", "BAK", "Lib", "Lic"]) {
        format!("Тип батареи → {s}")
    } else if let Some(a) = num("MUCHGC") {
        format!("Макс. ток заряда от сети → {} А", if a.is_empty() { "0".into() } else { a })
    } else if let Some(a) = num("MNCHGC").or_else(|| num("MCHGC")) {
        format!("Макс. общий ток заряда → {} А", if a.is_empty() { "0".into() } else { a })
    } else if let Some(x) = v("PBCV") {
        format!("Возврат на сеть → {x} В")
    } else if let Some(x) = v("PBDV") {
        format!("Возврат на батарею → {x} В")
    } else if let Some(x) = v("PSDV") {
        format!("Отсечка батареи → {x} В")
    } else if let Some(x) = v("PCVV") {
        format!("Напряжение заряда (CV) → {x} В")
    } else if let Some(x) = v("PBFT") {
        format!("Напряжение поддержки (float) → {x} В")
    } else if cmd == "F50" || cmd == "F60" {
        format!("Выходная частота → {} Гц", &cmd[1..])
    } else if let Some(f) = cmd.strip_prefix("PE") {
        format!("{} → вкл", f.chars().map(flag).collect::<Vec<_>>().join(", "))
    } else if let Some(f) = cmd.strip_prefix("PD") {
        format!("{} → выкл", f.chars().map(flag).collect::<Vec<_>>().join(", "))
    } else {
        String::new()
    };
    if r.is_empty() { format!("команда {cmd}") } else { format!("{r} ({cmd})") }
}

/// What changed between two automation lists, by rule id.
pub fn rules_diff(old: &[crate::automations::Rule], new: &[crate::automations::Rule]) -> String {
    let mut lines = Vec::new();
    for r in new {
        match old.iter().find(|o| o.id == r.id) {
            None => lines.push(format!("➕ «{}»", r.name)),
            Some(o) if o.enabled != r.enabled => {
                lines.push(format!("{} «{}»", if r.enabled { "▶️ включена" } else { "⏸ выключена" }, r.name))
            }
            Some(o) if serde_json::to_value(o).ok() != serde_json::to_value(r).ok() => lines.push(format!("✏️ «{}»", r.name)),
            _ => {}
        }
    }
    for o in old.iter().filter(|o| !new.iter().any(|r| r.id == o.id)) {
        lines.push(format!("🗑 «{}»", o.name));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::pi30;

    #[test]
    fn pi30_words() {
        assert_eq!(pi30("PCP03"), "Приоритет заряда → Только солнце (PCP03)");
        assert_eq!(pi30("POP02"), "Приоритет выхода → Батарея (SBU) (POP02)");
        assert_eq!(pi30("MUCHGC030"), "Макс. ток заряда от сети → 30 А (MUCHGC030)");
        assert_eq!(pi30("MNCHGC0060"), "Макс. общий ток заряда → 60 А (MNCHGC0060)");
        assert_eq!(pi30("PBCV25.0"), "Возврат на сеть → 25.0 В (PBCV25.0)");
        assert_eq!(pi30("PDa"), "Звуковой сигнал → выкл (PDa)");
        assert_eq!(pi30("F50"), "Выходная частота → 50 Гц (F50)");
        assert_eq!(pi30("QXYZ"), "команда QXYZ");
    }
}
