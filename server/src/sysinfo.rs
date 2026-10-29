//! Resource usage for the settings page: this process (RAM, CPU), the host it
//! runs on (inside a RouterOS container /proc shows the router itself), and
//! storage on the data volume. CPU is sampled in the background every 5 s.

use crate::{db, AppState};
use axum::{extract::State, response::IntoResponse, response::Response, Json};
use std::{
    path::Path,
    sync::Mutex,
    time::Instant,
};

#[derive(Default, Clone, Copy)]
pub struct Cpu {
    pub process_pct: Option<f64>, // % of one core
    pub host_pct: Option<f64>,    // % of all cores
    pub cores: usize,
}

pub struct Sampler {
    prev: Mutex<Option<(Instant, u64, u64, u64)>>, // (when, proc ticks, host total, host idle)
    pub cpu: Mutex<Cpu>,
    pub started: Instant,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler { prev: Mutex::new(None), cpu: Mutex::new(Cpu::default()), started: Instant::now() }
    }

    pub fn sample(&self) {
        let (Some(pt), Some((total, idle, cores))) = (proc_ticks(), host_ticks()) else { return };
        let now = Instant::now();
        let mut prev = self.prev.lock().unwrap();
        if let Some((t0, p0, tot0, idle0)) = *prev {
            let dt = now.duration_since(t0).as_secs_f64();
            let hz = clk_tck();
            let mut c = self.cpu.lock().unwrap();
            if dt > 0.0 {
                c.process_pct = Some((pt.saturating_sub(p0)) as f64 / hz / dt * 100.0);
            }
            let dtot = total.saturating_sub(tot0);
            if dtot > 0 {
                c.host_pct = Some(100.0 * (1.0 - idle.saturating_sub(idle0) as f64 / dtot as f64));
            }
            c.cores = cores;
        }
        *prev = Some((now, pt, total, idle));
    }
}

fn clk_tck() -> f64 {
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as f64 } else { 100.0 }
}

/// utime + stime of this process, in clock ticks.
fn proc_ticks() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

/// (total, idle+iowait, cores) from /proc/stat.
fn host_ticks() -> Option<(u64, u64, usize)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let mut lines = s.lines();
    let first: Vec<u64> = lines.next()?.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let cores = s.lines().filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(|b| b.is_ascii_digit())).count();
    Some((first.iter().sum(), first.get(3).copied().unwrap_or(0) + first.get(4).copied().unwrap_or(0), cores))
}

/// A "Key:   123 kB" value from a /proc file, in kB.
fn kb(path: &str, key: &str) -> Option<u64> {
    let s = std::fs::read_to_string(path).ok()?;
    s.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok()
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.metadata().map(|m| m.len()).unwrap_or(0)).sum()).unwrap_or(0)
}

/// (total, free) bytes of the filesystem holding `p`.
fn fs_space(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
}

pub async fn system(State(st): State<AppState>) -> Response {
    let cpu = *st.sys.cpu.lock().unwrap();
    let db_path = Path::new(&st.db_path);
    let data_dir = db_path.parent().unwrap_or(Path::new("."));
    let wal = |ext: &str| size(&db_path.with_file_name(format!("{}-{ext}", db_path.file_name().and_then(|n| n.to_str()).unwrap_or(""))));
    let (fs_total, fs_free) = fs_space(data_dir).unwrap_or((0, 0));
    let dbs = {
        let c = st.db.lock().unwrap();
        db::stats(&c).ok()
    };
    let load = std::fs::read_to_string("/proc/loadavg").ok()
        .and_then(|s| s.split_whitespace().next().map(String::from));
    let exe = std::env::current_exe().map(|p| size(&p)).unwrap_or(0);

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": st.sys.started.elapsed().as_secs(),
        "process": {
            "rss_kb": kb("/proc/self/status", "VmRSS:"),
            "rss_peak_kb": kb("/proc/self/status", "VmHWM:"),
            "cpu_pct": cpu.process_pct,
            "binary_bytes": exe,
        },
        "host": {
            "mem_total_kb": kb("/proc/meminfo", "MemTotal:"),
            "mem_avail_kb": kb("/proc/meminfo", "MemAvailable:"),
            "cpu_pct": cpu.host_pct,
            "cores": cpu.cores,
            "load1": load,
        },
        "storage": {
            "dir": data_dir.display().to_string(),
            "db_bytes": size(db_path),
            "wal_bytes": wal("wal") + wal("shm"),
            "ui_override_bytes": dir_size(&st.static_dir),
            "config_bytes": size(&data_dir.join("config.json")),
            "fs_total": fs_total,
            "fs_free": fs_free,
        },
        "db": dbs.map(|(rows, devices, oldest, newest)| serde_json::json!({
            "rows": rows, "devices": devices, "oldest_ts": oldest, "newest_ts": newest,
        })),
        "storage_policy": {
            "tiers": st.tiers.iter().map(|t| serde_json::json!({ "min_age_s": t.min_age, "res_s": t.res })).collect::<Vec<_>>(),
            "retention_days": st.retention_days,
        },
    }))
    .into_response()
}
