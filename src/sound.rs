use serde::Deserialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Reading {
    pub id: u32,
    pub name: String,
    pub percent: u32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Stream {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub outputs: Vec<Node>,
    #[serde(default)]
    pub sources: Vec<Node>,
    #[serde(default)]
    pub streams: Vec<Stream>,
    #[serde(default)]
    pub output: Option<Reading>,
    #[serde(default)]
    pub input: Option<Reading>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub schema: u32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct TestResult {
    pub seconds: u64,
    pub peak_dbfs: f64,
    pub rms_dbfs: f64,
    pub clipping: bool,
    pub verdict: String,
    #[serde(default)]
    pub file: Option<String>,
}

fn bounded_json(args: &[&str], timeout: Duration) -> Result<Vec<u8>, String> {
    let mut child = Command::new("niructl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start niructl: {error}"))?;
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
        }
    }
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(256 * 1024).read_to_end(&mut bytes);
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if timed_out {
        return Err("audio status timed out".into());
    }
    if !status.success() {
        return Err("niructl audio rejected the request".into());
    }
    Ok(bytes)
}

pub fn status() -> Result<Snapshot, String> {
    let bytes = bounded_json(&["audio", "status", "--json"], Duration::from_secs(3))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid audio status: {error}"))
}

pub fn microphone_test(seconds: u64) -> Result<TestResult, String> {
    let seconds = seconds.to_string();
    let bytes = bounded_json(
        &["audio", "mic-test", &seconds, "--json", "--keep"],
        Duration::from_secs(seconds.parse::<u64>().unwrap_or(5) + 10),
    )?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid microphone test: {error}"))
}

pub fn cleanup(path: &Option<String>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_contract() {
        let value = br#"{"schema":1,"available":true,"outputs":[{"id":57,"name":"Speakers","default":true}],"sources":[{"id":58,"name":"Mic","default":true}],"streams":[{"id":91,"name":"Firefox"}],"output":{"id":57,"name":"Speakers","percent":35,"muted":false},"input":{"id":58,"name":"Mic","percent":30,"muted":false}}"#;
        let snapshot: Snapshot = serde_json::from_slice(value).unwrap();
        assert!(snapshot.available);
        assert_eq!(snapshot.outputs[0].id, 57);
        assert_eq!(snapshot.input.unwrap().percent, 30);
        assert_eq!(snapshot.streams[0].name, "Firefox");
    }

    #[test]
    fn parses_test_contract() {
        let value = br#"{"schema":1,"seconds":5,"peak_dbfs":-10.0,"rms_dbfs":-20.0,"clipping":false,"verdict":"good","file":"/tmp/x.wav"}"#;
        let result: TestResult = serde_json::from_slice(value).unwrap();
        assert_eq!(result.verdict, "good");
        assert!(!result.clipping);
    }
}
