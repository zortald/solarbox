//! Runtime configuration in `config.json` on the data volume: boards, what the
//! UI may read/do on them, command whitelists, and UI metadata (labels…).
//! The file is re-read whenever its mtime changes — no restart. A broken file
//! keeps the previous config and reports the parse error via `/api/config`.

use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardCfg {
    pub name: String,
    pub host: String,
    #[serde(default = "d_web")]
    pub web_port: u16,
    #[serde(default = "d_ota")]
    pub ota_port: u16,
    /// UI-only fields (title, chip, restart/version entity paths, …).
    #[serde(flatten)]
    pub ui: serde_json::Map<String, serde_json::Value>,
}
fn d_web() -> u16 { 80 }
fn d_ota() -> u16 { 8266 }

/// One allowed control action. `board`/`entity` may be "*". `values`, when
/// present, restricts the value to one of the patterns (see `pattern_match`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionRule {
    #[serde(default = "star")]
    pub board: String,
    pub domain: String,
    #[serde(default = "star")]
    pub entity: String,
    pub actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}
fn star() -> String { "*".into() }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub boards: Vec<BoardCfg>,
    #[serde(default = "d_read")]
    pub read_domains: Vec<String>,
    #[serde(default = "d_actions")]
    pub actions: Vec<ActionRule>,
    /// Free-form UI settings (metric labels, page options) passed to the browser.
    #[serde(default)]
    pub ui: serde_json::Value,
    /// Local time for daily energy, "today" and time-based automations.
    #[serde(default)]
    pub timezone: crate::tz::Tz,
    /// Electricity tariffs for the cost counter.
    #[serde(default)]
    pub money: crate::money::Money,
}

fn d_read() -> Vec<String> {
    ["sensor", "binary_sensor", "text_sensor", "switch", "select", "number"].map(String::from).to_vec()
}

fn d_actions() -> Vec<ActionRule> {
    let r = |domain: &str, entity: &str, actions: &[&str], values: Option<&[&str]>| ActionRule {
        board: "*".into(),
        domain: domain.into(),
        entity: entity.into(),
        actions: actions.iter().map(|s| s.to_string()).collect(),
        values: values.map(|v| v.iter().map(|s| s.to_string()).collect()),
    };
    vec![
        r("switch", "*", &["turn_on", "turn_off", "toggle"], None),
        r("button", "*", &["press"], None),
        r("select", "*", &["set"], None),
        r("number", "*", &["set"], None),
        // PI30 commands the inverter board's own lambda also allows.
        r("text", "inverter command", &["set"], Some(&[
            "POP##", "PCP##", "PGR##", "PBT##", "MUCHGC###", "MNCHGC####", "MCHGC###",
            "PBCV##.#", "PBDV##.#", "PSDV##.#", "PCVV##.#", "PBFT##.#", "F50", "F60",
            "PE[abjkuvxyz]+", "PD[abjkuvxyz]+",
        ])),
    ]
}

impl Config {
    /// Built-in default; boards seeded from `SOLARBOX_BOARDS` if set.
    pub fn default_with(boards_env: &str) -> Self {
        let boards = crate::boards::parse_boards(boards_env)
            .into_iter()
            .map(|b| BoardCfg { name: b.name, host: b.host, web_port: b.web_port, ota_port: b.ota_port, ui: Default::default() })
            .collect();
        Config { boards, read_domains: d_read(), actions: d_actions(), ui: serde_json::json!({}), timezone: Default::default(), money: Default::default() }
    }

    pub fn board(&self, name: &str) -> Option<&BoardCfg> {
        self.boards.iter().find(|b| b.name == name)
    }

    /// Is `action` (with optional `value`) on `domain/entity` of `board` allowed?
    pub fn allows(&self, board: &str, domain: &str, entity: &str, action: &str, value: Option<&str>) -> bool {
        self.actions.iter().any(|r| {
            (r.board == "*" || r.board == board)
                && r.domain == domain
                && (r.entity == "*" || r.entity == entity)
                && r.actions.iter().any(|a| a == action)
                && match (&r.values, value) {
                    (None, _) => true,
                    (Some(pats), Some(v)) => pats.iter().any(|p| pattern_match(p, v)),
                    (Some(_), None) => false,
                }
        })
    }
}

/// Tiny pattern language for command whitelists:
/// `#` one digit, `@` one lowercase letter, `[abc]` one of these, `+` after a
/// token = one or more; anything else is a literal. Whole string must match.
pub fn pattern_match(pat: &str, s: &str) -> bool {
    #[derive(Clone)]
    enum Tok { Lit(char), Digit, Lower, Class(Vec<char>) }
    let mut toks: Vec<(Tok, bool)> = Vec::new();
    let mut it = pat.chars().peekable();
    while let Some(c) = it.next() {
        let t = match c {
            '#' => Tok::Digit,
            '@' => Tok::Lower,
            '[' => {
                let mut set = Vec::new();
                for x in it.by_ref() {
                    if x == ']' { break; }
                    set.push(x);
                }
                Tok::Class(set)
            }
            other => Tok::Lit(other),
        };
        let plus = it.peek() == Some(&'+');
        if plus { it.next(); }
        toks.push((t, plus));
    }
    let ok = |t: &Tok, c: char| match t {
        Tok::Lit(l) => *l == c,
        Tok::Digit => c.is_ascii_digit(),
        Tok::Lower => c.is_ascii_lowercase(),
        Tok::Class(set) => set.contains(&c),
    };
    fn rec(toks: &[(Tok, bool)], s: &[char], ok: &dyn Fn(&Tok, char) -> bool) -> bool {
        match toks.split_first() {
            None => s.is_empty(),
            Some(((t, plus), rest)) => {
                if s.is_empty() || !ok(t, s[0]) { return false; }
                if !plus { return rec(rest, &s[1..], ok); }
                let mut n = 1;
                while n < s.len() && ok(t, s[n]) { n += 1; }
                (1..=n).rev().any(|k| rec(rest, &s[k..], ok))
            }
        }
    }
    let chars: Vec<char> = s.chars().collect();
    rec(&toks, &chars, &ok)
}

/// Hot-reloading holder.
pub struct Store {
    path: PathBuf,
    inner: Mutex<(Option<SystemTime>, Arc<Config>, Option<String>)>,
}

impl Store {
    /// Load `path`, writing the default there first if it doesn't exist yet.
    pub fn open(path: PathBuf, default: Config) -> Self {
        if !path.exists() {
            if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
            match serde_json::to_string_pretty(&default) {
                Ok(s) => match std::fs::write(&path, s) {
                    Ok(_) => eprintln!("[config] wrote default {}", path.display()),
                    Err(e) => eprintln!("[config] can't write {}: {e}", path.display()),
                },
                Err(e) => eprintln!("[config] {e}"),
            }
        }
        let st = Store { path, inner: Mutex::new((None, Arc::new(default), None)) };
        st.get();
        st
    }

    /// Current config, re-read if the file changed since last time.
    pub fn get(&self) -> Arc<Config> {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        let mut g = self.inner.lock().unwrap();
        if mtime.is_some() && mtime != g.0 {
            g.0 = mtime;
            match std::fs::read_to_string(&self.path).map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Config>(&s).map_err(|e| e.to_string()))
            {
                Ok(c) => {
                    eprintln!("[config] loaded {} ({} board(s), {} rule(s))", self.path.display(), c.boards.len(), c.actions.len());
                    g.1 = Arc::new(c);
                    g.2 = None;
                }
                Err(e) => {
                    eprintln!("[config] {}: {e} — keeping previous config", self.path.display());
                    g.2 = Some(e);
                }
            }
        }
        g.1.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.inner.lock().unwrap().2.clone()
    }

    /// Validate and replace the file (the next `get` picks it up).
    pub fn save(&self, text: &str) -> Result<(), String> {
        let c: Config = serde_json::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
        let pretty = serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, pretty).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(pattern_match("PCP##", "PCP02"));
        assert!(!pattern_match("PCP##", "PCP2"));
        assert!(!pattern_match("PCP##", "PCP023"));
        assert!(pattern_match("PBCV##.#", "PBCV23.5"));
        assert!(!pattern_match("PBCV##.#", "PBCV23,5"));
        assert!(pattern_match("PE[abjkuvxyz]+", "PEa"));
        assert!(pattern_match("PE[abjkuvxyz]+", "PEabk"));
        assert!(!pattern_match("PE[abjkuvxyz]+", "PE"));
        assert!(!pattern_match("PE[abjkuvxyz]+", "PEq"));
        assert!(pattern_match("F50", "F50"));
        assert!(pattern_match("@+", "abc"));
    }

    #[test]
    fn default_rules() {
        let c = Config::default_with("inverter=192.168.88.10:80:8266");
        assert!(c.allows("inverter", "text", "inverter command", "set", Some("MNCHGC0060")));
        assert!(!c.allows("inverter", "text", "inverter command", "set", Some("PF")));
        assert!(!c.allows("inverter", "text", "inverter wifi connect", "set", Some("x")));
        assert!(c.allows("bms", "button", "Перезагрузка RST", "press", None));
        assert!(!c.allows("bms", "button", "x", "turn_on", None));
        assert_eq!(c.board("inverter").unwrap().ota_port, 8266);
    }
}
