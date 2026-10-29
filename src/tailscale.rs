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
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .map_err(|error| format!("tailscale is not available: {error}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
