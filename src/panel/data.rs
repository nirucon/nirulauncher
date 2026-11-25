//! What the panel's modules show. Everything is read locally: /proc and
//! /sys, the caches NIRUBAR and NIRUTODAY keep, NPLAY's control socket and
//! `niructl` status lines. Nothing here asks the network, so the panel opens
//! as fast offline as online. Slow sources run on worker threads.
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"))
}

// ------------------------------------------------------------------ system

/// Busy and total jiffies of all CPUs, from the first line of /proc/stat.
pub(crate) fn cpu_times() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    parse_cpu(text.lines().next()?)
}

fn parse_cpu(line: &str) -> Option<(u64, u64)> {
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    let total: u64 = v.iter().sum();
    // idle and iowait are not busy.
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

/// Percent busy between two readings.
pub(crate) fn cpu_percent(a: (u64, u64), b: (u64, u64)) -> f64 {
    let busy = b.0.saturating_sub(a.0) as f64;
    let total = b.1.saturating_sub(a.1) as f64;
    if total <= 0.0 {
        0.0
    } else {
        (busy / total * 100.0).clamp(0.0, 100.0)
    }
}

/// (used, total) bytes of memory.
pub(crate) fn memory() -> Option<(u64, u64)> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    Some((total.saturating_sub(available), total))
}

/// (used, total) bytes of the file system holding `path`.
pub(crate) fn disk(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid C string and a zeroed out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize as u64;
    let total = s.f_blocks as u64 * f;
    let free = s.f_bavail as u64 * f;
    Some((total.saturating_sub(free), total))
}

/// The hottest CPU package or core sensor, °C.
pub(crate) fn temperature() -> Option<f64> {
    let mut best: Option<f64> = None;
    let dir = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in dir.flatten() {
        let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        // The CPU's own sensors only: an ACPI thermal zone (acpitz) can
        // take seconds to read on some machines.
        if !matches!(
            name.trim(),
            "coretemp" | "k10temp" | "zenpower" | "cpu_thermal"
        ) {
            continue;
        }
        for i in 1..=8 {
            let Ok(t) = std::fs::read_to_string(entry.path().join(format!("temp{i}_input"))) else {
                continue;
            };
            if let Ok(m) = t.trim().parse::<f64>() {
                let c = m / 1000.0;
                if (1.0..130.0).contains(&c) {
                    best = Some(best.map_or(c, |b: f64| b.max(c)));
                }
            }
        }
    }
    best
}

/// Battery percent and whether it charges, when there is a battery.
pub(crate) fn battery() -> Option<(u8, bool)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let p = entry.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        let cap = std::fs::read_to_string(p.join("capacity")).ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some((
            cap.trim().parse::<u8>().ok()?.min(100),
            matches!(status.trim(), "Charging" | "Full"),
        ));
    }
    None
}

/// Seconds since boot.
pub(crate) fn uptime() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/uptime").ok()?;
    t.split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .map(|s| s as u64)
}

/// The system figures that change, read together on a worker: some
/// sensors are slow to read.
#[derive(Clone, Default)]
pub(crate) struct System {
    pub(crate) cpu: f64,
    pub(crate) memory: Option<(u64, u64)>,
    pub(crate) disk: Option<(u64, u64)>,
    pub(crate) temperature: Option<f64>,
    pub(crate) battery: Option<(u8, bool)>,
    pub(crate) uptime: Option<u64>,
}

/// Read them, with the CPU measured over `over`.
pub(crate) fn system(over: Duration) -> System {
    let a = cpu_times();
    std::thread::sleep(over);
    let b = cpu_times();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    System {
        cpu: match (a, b) {
            (Some(a), Some(b)) => cpu_percent(a, b),
            _ => 0.0,
        },
        memory: memory(),
        disk: disk(&home),
        temperature: temperature(),
        battery: battery(),
        uptime: uptime(),
    }
}

pub(crate) fn human_bytes(b: u64) -> String {
    let g = b as f64 / 1024.0 / 1024.0 / 1024.0;
    if g >= 10.0 {
        format!("{g:.0} GB")
    } else if g >= 1.0 {
        format!("{g:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1024.0 / 1024.0)
    }
}

pub(crate) fn human_uptime(s: u64) -> String {
    let (d, h, m) = (s / 86400, s / 3600 % 24, s / 60 % 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

// ----------------------------------------------------------------- updates

pub(crate) struct Updates {
    pub(crate) installed: Option<String>,
    /// A newer NIRUX release, from the bar's daily check.
    pub(crate) newer: Option<String>,
    /// OS package updates the bar counted.
    pub(crate) packages: Option<u32>,
}

/// Read the bar's caches; nothing is removed or fetched (the launcher's
/// own reminder takes the NIRUX answer once, the panel only looks).
pub(crate) fn updates() -> Updates {
    let installed = crate::providers::system::installed_nirux_version();
    let bar = cache_home().join("nirubar");
    let newer = std::fs::read_to_string(bar.join("nirux-update"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|latest| {
            installed
                .as_deref()
                .is_some_and(|i| crate::providers::system::is_newer_version(latest, i))
        });
    let packages = std::fs::read_to_string(bar.join("updates"))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    Updates {
        installed,
        newer,
        packages,
    }
}

// ------------------------------------------------------------------- today

/// NIRUTODAY's day in brief (`nirutoday --brief`, from its cache).
#[derive(Default, Clone)]
pub(crate) struct Today {
    pub(crate) json: Option<serde_json::Value>,
}

pub(crate) fn today() -> Today {
    let out = bounded(&["nirutoday", "--brief"], Duration::from_millis(1500));
    Today {
        json: out.and_then(|t| serde_json::from_str(t.trim()).ok()),
    }
}

// ------------------------------------------------------------------- media

#[derive(Default, Clone, PartialEq)]
pub(crate) struct Media {
    pub(crate) playing: bool,
    pub(crate) title: String,
    pub(crate) artist: String,
}

fn nplay_socket() -> PathBuf {
    let get = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(p) = get("NPLAY_SERVICE_SOCKET") {
        return p;
    }
    get("XDG_RUNTIME_DIR")
        .map(|d| d.join("nplay"))
        .or_else(|| get("XDG_STATE_HOME").map(|d| d.join("nplay/run")))
        .unwrap_or_else(|| home().join(".local/state/nplay/run"))
        .join("control.sock")
}

/// One JSON-RPC call to NPLAY; `None` when it is not running.
pub(crate) fn nplay(method: &str) -> Option<serde_json::Value> {
    let sock = std::os::unix::net::UnixStream::connect(nplay_socket()).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(600)))
        .ok()?;
    sock.set_write_timeout(Some(Duration::from_millis(600)))
        .ok()?;
    let mut w = sock.try_clone().ok()?;
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}});
    w.write_all(format!("{req}\n").as_bytes()).ok()?;
    let mut reader = BufReader::new(sock);
    // Events may come first; the answer carries our id.
    for _ in 0..20 {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        if v.get("id") == Some(&serde_json::json!(1)) {
            return v.get("result").cloned();
        }
    }
    None
}

pub(crate) fn media() -> Option<Media> {
    parse_media(&nplay("state.snapshot")?)
}

fn parse_media(v: &serde_json::Value) -> Option<Media> {
    let st = v.get("state")?;
    let status = st.get("status").and_then(|s| s.as_str()).unwrap_or("");
    let meta = st.get("metadata");
    let text = |k: &str| {
        meta.and_then(|m| m.get(k))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string()
    };
    let title = text("title");
    if title.is_empty() && status != "playing" {
        return None;
    }
    Some(Media {
        playing: status == "playing",
        title,
        artist: text("artist"),
    })
}

// --------------------------------------------------------------- computers

/// Whether a program is on PATH (a few `stat` calls, no process).
pub(crate) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(dir.join(program))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// The tailnet's computers from the local Tailscale daemon (`tailscale
/// status --json`, at most 1.5 seconds). No network request of its own.
pub(crate) fn computers() -> Result<Vec<crate::tailscale::Device>, String> {
    let out = bounded_bytes(
        &["tailscale", "status", "--json"],
        Duration::from_millis(1500),
    )
    .ok_or_else(|| "Tailscale did not answer: is it running?".to_string())?;
    crate::tailscale::devices(&out)
}

/// Seconds since the epoch, now.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

// ----------------------------------------------------------------- toggles

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Toggle {
    Dnd,
    NightLight,
    Wifi,
    Bluetooth,
}

impl Toggle {
    pub(crate) const ALL: [Toggle; 4] = [
        Toggle::Dnd,
        Toggle::NightLight,
        Toggle::Wifi,
        Toggle::Bluetooth,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Toggle::Dnd => "Do not disturb",
            Toggle::NightLight => "Night light",
            Toggle::Wifi => "Wi-Fi",
            Toggle::Bluetooth => "Bluetooth",
        }
    }

    /// On or off now; `None` when it cannot be told (no adapter, no tool).
    pub(crate) fn state(self) -> Option<bool> {
        match self {
            Toggle::Dnd => parse_on_off(&bounded(&["niructl", "dnd", "status"], SHORT)?),
            Toggle::NightLight => {
                let out = bounded(&["niructl", "nightlight", "status"], SHORT)?;
                Some(nightlight_on(&out))
            }
            Toggle::Wifi => match bounded(&["nmcli", "radio", "wifi"], SHORT)?.trim() {
                "enabled" => Some(true),
                "disabled" => Some(false),
                _ => None,
            },
            Toggle::Bluetooth => {
                let out = bounded(&["niructl", "bluetooth", "status"], SHORT)?;
                bluetooth_on(&out)
            }
        }
    }

    /// The command that sets it (`on`: the new state).
    pub(crate) fn command(self, on: bool) -> Vec<String> {
        let word = if on { "on" } else { "off" };
        let v: Vec<&str> = match self {
            Toggle::Dnd => vec!["niructl", "dnd", word],
            Toggle::NightLight => vec!["niructl", "nightlight", word],
            Toggle::Wifi => vec!["nmcli", "radio", "wifi", word],
            Toggle::Bluetooth => vec!["niructl", "bluetooth", "power", word],
        };
        v.into_iter().map(str::to_string).collect()
    }
}

const SHORT: Duration = Duration::from_millis(1500);

fn parse_on_off(text: &str) -> Option<bool> {
    let t = text.trim().to_ascii_lowercase();
    let first = t.split_whitespace().last().unwrap_or("");
    if t.contains("on") && !t.contains("off") || first == "on" {
        Some(true)
    } else if t.contains("off") {
        Some(false)
    } else {
        None
    }
}

fn nightlight_on(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("").to_ascii_lowercase();
    !(first.contains("off") || first.contains("disabled"))
}

fn bluetooth_on(text: &str) -> Option<bool> {
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("state=") {
            return match v.trim() {
                "on" | "powered" => Some(true),
                "off" => Some(false),
                _ => None,
            };
        }
        if let Some(v) = line.trim().strip_prefix("power=") {
            return Some(v.trim() == "on");
        }
    }
    None
}

/// Run a helper with a deadline; its stdout, or `None` when it is missing,
/// fails or takes too long (it is then killed).
pub(crate) fn bounded(argv: &[&str], limit: Duration) -> Option<String> {
    bounded_bytes(argv, limit).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `bounded`, the output as bytes. It is read while the helper runs, so a
/// large answer cannot fill the pipe and stall it.
pub(crate) fn bounded_bytes(argv: &[&str], limit: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + limit;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        use std::io::Read;
        stdout.read_to_end(&mut out).ok().map(|_| out)
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()??;
                return status.success().then_some(out);
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(15));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_and_memory_are_read() {
        let a = parse_cpu("cpu  100 0 100 800 0 0 0 0 0 0").unwrap();
        let b = parse_cpu("cpu  150 0 150 900 0 0 0 0 0 0").unwrap();
        assert_eq!(a, (200, 1000));
        assert!((cpu_percent(a, b) - 50.0).abs() < 1e-9);
        assert_eq!(cpu_percent(b, b), 0.0);
        let m =
            parse_meminfo("MemTotal:       16000 kB\nMemFree: 1 kB\nMemAvailable:    4000 kB\n");
        assert_eq!(m, Some((12000 * 1024, 16000 * 1024)));
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
        assert_eq!(human_bytes(15 * 1024 * 1024 * 1024 + 1), "15 GB");
        assert_eq!(human_uptime(90061), "1 d 1 h");
        assert_eq!(human_uptime(3700), "1 h 1 min");
    }

    #[test]
    fn toggle_states_are_read_from_their_tools() {
        assert_eq!(parse_on_off("on\n"), Some(true));
        assert_eq!(parse_on_off("dnd off"), Some(false));
        assert_eq!(parse_on_off(""), None);
        assert!(nightlight_on("mode: gradual\nnight of ..."));
        assert!(!nightlight_on("mode: off"));
        assert_eq!(bluetooth_on("state=on\n"), Some(true));
        assert_eq!(bluetooth_on("state=off"), Some(false));
        assert_eq!(bluetooth_on("state=unavailable\nreason=x"), None);
        assert_eq!(
            Toggle::Wifi.command(false),
            ["nmcli", "radio", "wifi", "off"]
        );
    }

    #[test]
    fn media_comes_from_the_snapshot() {
        let v = serde_json::json!({"state": {"status": "playing",
            "metadata": {"title": "Song", "artist": "Band"}}});
        let m = parse_media(&v).unwrap();
        assert!(m.playing && m.title == "Song" && m.artist == "Band");
        let idle = serde_json::json!({"state": {"status": "stopped", "metadata": {}}});
        assert!(parse_media(&idle).is_none());
    }
}
