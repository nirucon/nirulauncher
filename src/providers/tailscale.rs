//! Tailscale peer discovery for the launcher. No shell and no network request:
//! `tailscale status --json` is the local daemon's own snapshot.

use serde_json::Value;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Peer {
    pub host: String,
    pub address: String,
}

pub fn peers() -> Result<Vec<Peer>, String> {
    let output = Command::new("timeout")
        .args(["-k", "0.2", "0.5", "tailscale", "status", "--json"])
        .output()
        .map_err(|error| format!("tailscale is not available: {error}"))?;
    if output.status.code() == Some(124) {
        return Err("tailscale status did not respond within 500 ms".into());
    }
    if !output.status.success() {
        return Err("tailscale is not running or not logged in".into());
    }
    parse(&output.stdout)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Peer>, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("tailscale JSON: {error}"))?;
    let mut peers = Vec::new();
    if let Some(object) = value.get("Peer").and_then(Value::as_object) {
        for peer in object.values() {
            if !peer.get("Online").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let host = string(peer, "DNSName")
                .map(|name| name.trim_end_matches('.').to_string())
                .filter(|name| !name.is_empty())
                .or_else(|| string(peer, "HostName"))
                .unwrap_or_else(|| "unknown".into());
            let address = peer
                .get("TailscaleIPs")
                .and_then(Value::as_array)
                .and_then(|ips| ips.first())
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            peers.push(Peer { host, address });
        }
    }
    peers.sort_by(|a, b| a.host.cmp(&b.host));
    Ok(peers)
}

/// One computer in the tailnet, for NIRUPANEL's computers module.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// The MagicDNS name's first label (what `ssh NAME` reaches), else the
    /// host name.
    pub name: String,
    /// The machine's own host name (may differ from `name`).
    pub host_name: String,
    /// What to connect to: `name` with MagicDNS, else the Tailscale IP.
    pub target: String,
    /// The first Tailscale IPv4 address, else the first address.
    pub ip: String,
    pub os: String,
    pub online: bool,
    /// This computer (`Self`).
    pub this: bool,
    /// Owned by the same Tailscale user as this computer.
    pub mine: bool,
    /// Last seen, seconds since the epoch (offline peers).
    pub last_seen: Option<i64>,
    /// This computer sends its traffic through this peer.
    pub exit_node: bool,
    /// `Some(true)` direct, `Some(false)` through a relay; `None` unknown.
    pub direct: Option<bool>,
    /// The DERP relay region (for example `ams`).
    pub relay: String,
}

/// Seconds since the epoch for an RFC 3339 time (`2026-11-22T12:26:44.1Z`,
/// `…+01:00`); `None` for Tailscale's "never" (year 1) and anything else
/// before 1970 or unreadable.
pub fn epoch_seconds(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if y < 1970 || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 {
        return None;
    }
    // The offset after the optional fraction.
    let rest = text[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.as_bytes().first() {
        Some(b'Z') | Some(b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let oh = rest.get(1..3)?.parse::<i64>().ok()?;
            let om = rest.get(4..6)?.parse::<i64>().ok()?;
            let o = oh * 3600 + om * 60;
            if *sign == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let (yy, m) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s - offset)
}

/// "just now", "5 min ago", "2 h ago", "3 d ago", "8 weeks ago".
pub fn ago(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    if s < 60 {
        "just now".into()
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else if s < 86_400 {
        format!("{} h ago", s / 3600)
    } else if s < 86_400 * 14 {
        format!("{} d ago", s / 86_400)
    } else if s < 86_400 * 365 {
        format!("{} weeks ago", s / (86_400 * 7))
    } else {
        "over a year ago".into()
    }
}

fn device(v: &Value, this: bool, me: Option<i64>, magic: bool) -> Device {
    let host_name = string(v, "HostName").unwrap_or_default();
    let name = string(v, "DNSName")
        .and_then(|n| n.split('.').next().map(str::to_string))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| host_name.clone());
    let ips: Vec<String> = v
        .get("TailscaleIPs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let ip = ips
        .iter()
        .find(|ip| ip.contains('.'))
        .or_else(|| ips.first())
        .cloned()
        .unwrap_or_default();
    let online = this || v.get("Online").and_then(Value::as_bool).unwrap_or(false);
    let user = v.get("UserID").and_then(Value::as_i64);
    let target = if magic && !name.is_empty() {
        name.clone()
    } else if !ip.is_empty() {
        ip.clone()
    } else {
        name.clone()
    };
    Device {
        target,
        host_name,
        ip,
        os: string(v, "OS").unwrap_or_default(),
        online,
        this,
        mine: this || (user.is_some() && user == me),
        last_seen: string(v, "LastSeen").and_then(|t| epoch_seconds(&t)),
        exit_node: v.get("ExitNode").and_then(Value::as_bool).unwrap_or(false),
        direct: (online && !this).then(|| string(v, "CurAddr").is_some_and(|a| !a.is_empty())),
        relay: string(v, "Relay").unwrap_or_default(),
        name,
    }
}

/// Every computer in `tailscale status --json`: this one first, then the
/// peers in the JSON's order. An error when Tailscale is not up.
pub fn devices(bytes: &[u8]) -> Result<Vec<Device>, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("tailscale JSON: {error}"))?;
    match value.get("BackendState").and_then(Value::as_str) {
        Some("Running") | None => {}
        Some("NeedsLogin") => return Err("Tailscale is logged out".into()),
        Some(_) => return Err("Tailscale is not connected".into()),
    }
    let me = value.get("Self");
    let my_user = me.and_then(|s| s.get("UserID")).and_then(Value::as_i64);
    let magic = value
        .get("MagicDNSSuffix")
        .and_then(Value::as_str)
        .map_or(true, |s| !s.is_empty());
    let mut out = Vec::new();
    if let Some(s) = me {
        out.push(device(s, true, my_user, magic));
    }
    if let Some(peers) = value.get("Peer").and_then(Value::as_object) {
        out.extend(peers.values().map(|p| device(p, false, my_user, magic)));
    }
    Ok(out)
}

/// The computers to show. With `hosts`, those in that order (by name or
/// host name, any case); otherwise this computer and the peers of the same
/// user, the ones online first by name, then the offline ones last seen
/// most recently first. `hide` leaves names out.
pub fn pick(devices: &[Device], hosts: &[String], hide: &[String]) -> Vec<Device> {
    let is = |d: &Device, n: &str| {
        d.name.eq_ignore_ascii_case(n.trim()) || d.host_name.eq_ignore_ascii_case(n.trim())
    };
    let mut out: Vec<Device> = if hosts.is_empty() {
        let mut v: Vec<Device> = devices.iter().filter(|d| d.mine).cloned().collect();
        v.sort_by(|a, b| {
            b.this
                .cmp(&a.this)
                .then(b.online.cmp(&a.online))
                .then_with(|| {
                    if a.online {
                        a.name.to_lowercase().cmp(&b.name.to_lowercase())
                    } else {
                        b.last_seen.cmp(&a.last_seen)
                    }
                })
        });
        v
    } else {
        let mut v: Vec<Device> = Vec::new();
        for h in hosts {
            if let Some(d) = devices.iter().find(|d| is(d, h)) {
                if !v.iter().any(|x| x.name == d.name && x.ip == d.ip) {
                    v.push(d.clone());
                }
            }
        }
        v
    };
    out.retain(|d| !hide.iter().any(|h| is(d, h)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &[u8] = br#"{
      "BackendState": "Running",
      "MagicDNSSuffix": "ex.ts.net",
      "Self": {"HostName":"box","DNSName":"box.ex.ts.net.","OS":"linux","UserID":7,
        "TailscaleIPs":["fd7a::1","100.64.0.1"],"Online":true,"CurAddr":"","Relay":"ams",
        "LastSeen":"0001-01-01T00:00:00Z"},
      "Peer": {
        "k1": {"HostName":"L-5CD4","DNSName":"winwork.ex.ts.net.","OS":"windows","UserID":7,
          "TailscaleIPs":["100.64.0.5"],"Online":false,"CurAddr":"","Relay":"fra",
          "LastSeen":"2026-11-25T10:00:00Z"},
        "k2": {"HostName":"othala","DNSName":"othala.ex.ts.net.","OS":"linux","UserID":7,
          "TailscaleIPs":["100.64.0.2"],"Online":true,"CurAddr":"1.2.3.4:41641","Relay":"ams",
          "ExitNode":true,"LastSeen":"0001-01-01T00:00:00Z"},
        "k3": {"HostName":"mac","DNSName":"mac.ex.ts.net.","OS":"macOS","UserID":7,
          "TailscaleIPs":["100.64.0.3"],"Online":false,"Relay":"hel",
          "LastSeen":"2026-11-25T11:30:00.5+01:00"},
        "k4": {"HostName":"alpha","DNSName":"alpha.ex.ts.net.","OS":"linux","UserID":7,
          "TailscaleIPs":["100.64.0.4"],"Online":true,"CurAddr":"","Relay":"ams"},
        "k5": {"HostName":"friend","DNSName":"friend.ex.ts.net.","OS":"linux","UserID":99,
          "TailscaleIPs":["100.64.0.9"],"Online":true}
      }
    }"#;

    #[test]
    fn devices_come_from_the_status_json() {
        let all = devices(STATUS).unwrap();
        assert_eq!(all.len(), 6);
        let me = &all[0];
        assert!(me.this && me.online && me.mine);
        assert_eq!((me.name.as_str(), me.ip.as_str()), ("box", "100.64.0.1"));
        assert_eq!(me.last_seen, None, "year 1 is never");
        let win = all.iter().find(|d| d.name == "winwork").unwrap();
        assert_eq!(win.host_name, "L-5CD4");
        assert_eq!(win.target, "winwork");
        assert!(!win.online && win.direct.is_none());
        assert_eq!(win.last_seen, epoch_seconds("2026-11-25T10:00:00Z"));
        let othala = all.iter().find(|d| d.name == "othala").unwrap();
        assert!(othala.exit_node && othala.direct == Some(true));
        let alpha = all.iter().find(|d| d.name == "alpha").unwrap();
        assert_eq!((alpha.direct, alpha.relay.as_str()), (Some(false), "ams"));
        assert!(!all.iter().find(|d| d.name == "friend").unwrap().mine);
        // No MagicDNS: connect to the address.
        let plain = br#"{"BackendState":"Running","MagicDNSSuffix":"",
          "Self":{"HostName":"box","DNSName":"","UserID":1,"TailscaleIPs":["100.64.0.1"]}}"#;
        assert_eq!(devices(plain).unwrap()[0].target, "100.64.0.1");
        let stopped = br#"{"BackendState":"Stopped","Self":{}}"#;
        assert_eq!(devices(stopped).unwrap_err(), "Tailscale is not connected");
        assert!(devices(br#"{"BackendState":"NeedsLogin"}"#).is_err());
        assert!(devices(b"not json").is_err());
    }

    #[test]
    fn own_computers_are_picked_and_ordered() {
        let all = devices(STATUS).unwrap();
        let names = |v: Vec<Device>| v.into_iter().map(|d| d.name).collect::<Vec<_>>();
        // This one, online by name, offline by last seen (mac 10:30 UTC
        // after winwork 10:00 UTC); another user's peer is left out.
        assert_eq!(
            names(pick(&all, &[], &[])),
            ["box", "alpha", "othala", "mac", "winwork"]
        );
        assert_eq!(
            names(pick(&all, &[], &["OTHALA".into(), "box".into()])),
            ["alpha", "mac", "winwork"]
        );
        let hosts: Vec<String> = ["othala", "L-5CD4", "friend", "missing", "othala"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            names(pick(&all, &hosts, &[])),
            ["othala", "winwork", "friend"]
        );
    }

    #[test]
    fn times_and_ages_read_as_people_say_them() {
        assert_eq!(epoch_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_seconds("2026-11-25T00:00:00Z"), Some(1_795_564_800));
        assert_eq!(
            epoch_seconds("2026-11-25T01:00:00+01:00"),
            epoch_seconds("2026-11-25T00:00:00Z")
        );
        assert_eq!(
            epoch_seconds("2026-11-24T23:00:00.123456-01:00"),
            epoch_seconds("2026-11-25T00:00:00Z")
        );
        assert_eq!(epoch_seconds("0001-01-01T00:00:00Z"), None);
        assert_eq!(epoch_seconds("yesterday"), None);
        assert_eq!(ago(100, 90), "just now");
        assert_eq!(ago(10_000, 10_000 - 300), "5 min ago");
        assert_eq!(ago(100_000, 100_000 - 7200), "2 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 d ago");
        assert_eq!(ago(100_000_000, 100_000_000 - 56 * 86_400), "8 weeks ago");
        assert_eq!(ago(100_000_000, 0), "over a year ago");
    }

    #[test]
    fn online_peers_are_sorted_and_offline_peers_are_ignored() {
        let body = br#"{
          "Peer": {
            "b": {"HostName":"beta","DNSName":"beta.example.ts.net.","Online":true,"TailscaleIPs":["100.64.0.2"]},
            "a": {"HostName":"alpha","DNSName":"alpha.example.ts.net.","Online":true,"TailscaleIPs":["100.64.0.1"]},
            "x": {"HostName":"offline","Online":false}
          }
        }"#;
        let peers = parse(body).unwrap();
        assert_eq!(
            peers
                .iter()
                .map(|peer| peer.host.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha.example.ts.net", "beta.example.ts.net"]
        );
        assert_eq!(peers[0].address, "100.64.0.1");
    }
}
