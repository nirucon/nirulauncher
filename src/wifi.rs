//! Wi-Fi discovery and control for NIRU Launcher.
//!
//! NetworkManager is driven through its system D-Bus API. Standalone
//! wpa_supplicant uses its own control interface; read-only state is also
//! available through `iw` so the UI remains useful if the control socket is
//! not reachable. The module never starts a network manager and never guesses
//! ownership from the presence of a command.
//!
//! Secrets are only passed on stdin to wpa_supplicant. They are never placed in
//! argv, logs, the serialized snapshot, or launcher history.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
#[path = "wifi_native.rs"]
mod native;

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const NM_WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const NM_AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const NM_IP4: &str = "org.freedesktop.NetworkManager.IP4Config";
const NM_SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";

const AP_PRIVACY: u32 = 0x1;
const AP_KEY_PSK: u32 = 0x100;
const AP_KEY_EAP: u32 = 0x200;
const AP_KEY_SAE: u32 = 0x400;
const AP_KEY_OWE: u32 = 0x800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // These are the real backend names.
pub enum Manager {
    NetworkManager,
    WpaSupplicant,
    Iwd,
    ConnMan,
    None,
}

impl Manager {
    pub fn label(self) -> &'static str {
        match self {
            Manager::NetworkManager => "NetworkManager",
            Manager::WpaSupplicant => "wpa_supplicant",
            Manager::Iwd => "iwd",
            Manager::ConnMan => "ConnMan",
            Manager::None => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub present: bool,
    pub interface: String,
    pub manager: String,
    pub powered: bool,
    pub hard_blocked: bool,
    pub soft_blocked: bool,
    pub connected: bool,
    pub ssid: String,
    pub signal: Option<u8>,
    pub security: String,
    pub address: String,
    pub state: String,
    pub detail: String,
    pub error: String,
    pub networks: Vec<Network>,
    pub saved: Vec<String>,
    pub profiles: Vec<SavedProfile>,
    pub active_profile: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SavedProfile {
    pub id: String,
    pub ssid: String,
    pub security: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Network {
    pub ssid: String,
    pub signal: Option<u8>,
    pub security: String,
    pub frequency: Option<u32>,
    pub bssid: Option<String>,
    pub saved: bool,
    pub active: bool,
    pub hidden: bool,
}

impl Network {
    pub fn needs_secret(&self) -> bool {
        !matches!(self.security.as_str(), "Open" | "OWE" | "Unknown" | "Saved")
            && !self.security.contains("Enterprise")
    }

    pub fn can_connect(&self) -> bool {
        !self.security.contains("Enterprise") && self.security != "WEP"
    }
}

/// True when the kernel exposes a wireless interface, even if its radio is off.
pub fn hardware_present() -> bool {
    wireless_interface().is_some()
}

pub fn snapshot(scan: bool) -> Snapshot {
    let Some(interface) = wireless_interface() else {
        return Snapshot {
            manager: Manager::None.label().into(),
            detail: "No Wi-Fi hardware found".into(),
            ..Default::default()
        };
    };
    let (manager, blocked) = manager_for(&interface);
    let mut snap = Snapshot {
        present: true,
        interface: interface.clone(),
        manager: manager.label().into(),
        powered: !blocked.0,
        hard_blocked: blocked.0,
        soft_blocked: blocked.1,
        state: if blocked.0 {
            "blocked"
        } else if blocked.1 {
            "off"
        } else {
            "disconnected"
        }
        .into(),
        ..Default::default()
    };
    let result = match manager {
        Manager::NetworkManager => nm_snapshot(&interface, scan),
        Manager::WpaSupplicant => wpa_snapshot(&interface, scan),
        Manager::Iwd | Manager::ConnMan => native::snapshot(&interface, manager, scan),
        Manager::None => Err("No supported Wi-Fi manager owns this interface".into()),
    };
    match result {
        Ok(mut value) => {
            value.present = true;
            value.interface = interface;
            value.manager = manager.label().into();
            value.hard_blocked = blocked.0;
            value.soft_blocked = blocked.1;
            if blocked.0 || blocked.1 {
                value.powered = false;
                value.connected = false;
                value.ssid.clear();
                value.signal = None;
                value.state = if blocked.0 { "blocked" } else { "off" }.into();
            }
            value
        }
        Err(error) => {
            snap.error = error;
            snap.detail = format!("{} is not reachable", manager.label());
            snap
        }
    }
}

fn manager_for(interface: &str) -> (Manager, (bool, bool)) {
    let blocked = rfkill_state();
    if let Ok(conn) = Connection::system() {
        if name_has_owner(&conn, NM) {
            return (
                if nm_device_path(&conn, interface).is_ok() {
                    Manager::NetworkManager
                } else {
                    Manager::None
                },
                blocked,
            );
        }
        if name_has_owner(&conn, "net.connman.iwd") && native::owns(interface, Manager::Iwd) {
            return (Manager::Iwd, blocked);
        }
        if name_has_owner(&conn, "net.connman") && native::owns(interface, Manager::ConnMan) {
            return (Manager::ConnMan, blocked);
        }
    }
    let sock = Path::new("/run/wpa_supplicant").join(interface);
    if sock.exists() && wpa_process_running() {
        return (Manager::WpaSupplicant, blocked);
    }
    (Manager::None, blocked)
}

fn name_has_owner(conn: &Connection, name: &str) -> bool {
    let Ok(proxy) = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return false;
    };
    proxy
        .call::<_, _, bool>("NameHasOwner", &(name,))
        .unwrap_or(false)
}

fn nm_device_path(conn: &Connection, interface: &str) -> Result<OwnedObjectPath, String> {
    let nm = proxy(conn, NM, NM_PATH, NM)?;
    let devices: Vec<OwnedObjectPath> = nm
        .call("GetDevices", &())
        .map_err(|e| format!("NetworkManager devices: {e}"))?;
    for path in devices {
        let props = get_all(conn, NM, path.as_str(), NM_DEVICE)?;
        if string_prop(&props, "Interface").as_deref() == Some(interface) {
            if !bool_prop(&props, "Managed").unwrap_or(false) {
                return Err(format!("NetworkManager leaves {interface} unmanaged"));
            }
            return Ok(path);
        }
    }
    Err(format!("NetworkManager does not manage {interface}"))
}

fn nm_snapshot(interface: &str, scan: bool) -> Result<Snapshot, String> {
    let conn = Connection::system().map_err(|e| format!("system D-Bus: {e}"))?;
    let manager = get_all(&conn, NM, NM_PATH, NM)?;
    let wireless_enabled = bool_prop(&manager, "WirelessEnabled").unwrap_or(true);
    let hardware_enabled = bool_prop(&manager, "WirelessHardwareEnabled").unwrap_or(true);
    let device = nm_device_path(&conn, interface)?;
    let props = get_all(&conn, NM, device.as_str(), NM_DEVICE)?;
    let state = u32_prop(&props, "State").unwrap_or(0);
    let active = path_prop(&props, "ActiveConnection");
    let mut snap = Snapshot {
        present: true,
        interface: interface.into(),
        manager: "NetworkManager".into(),
        powered: wireless_enabled && hardware_enabled,
        connected: state == 100 && active.is_some(),
        state: nm_state_name(state).into(),
        detail: "NetworkManager".into(),
        ..Default::default()
    };

    let saved = nm_saved(&conn).unwrap_or_default();
    snap.saved = saved.iter().map(|(ssid, _, _)| ssid.clone()).collect();
    snap.profiles = saved
        .iter()
        .map(|(ssid, security, path)| SavedProfile {
            id: path.to_string(),
            ssid: ssid.clone(),
            security: security.clone(),
        })
        .collect();
    snap.saved.sort();

    if scan {
        if let Ok(wireless) = proxy(&conn, NM, device.as_str(), NM_WIRELESS) {
            let options: HashMap<String, OwnedValue> = HashMap::new();
            if let Err(error) = wireless.call::<_, _, ()>("RequestScan", &(options,)) {
                snap.error = format!("Network scan: {error}");
            } else {
                std::thread::sleep(Duration::from_millis(1200));
            }
        }
    }

    let mut networks: HashMap<(String, String), Network> = HashMap::new();
    if let Ok(wireless) = proxy(&conn, NM, device.as_str(), NM_WIRELESS) {
        if let Ok(aps) = wireless.call::<_, _, Vec<OwnedObjectPath>>("GetAccessPoints", &()) {
            for ap in aps {
                if let Ok(props) = get_all(&conn, NM, ap.as_str(), NM_AP) {
                    let Some(ssid) = bytes_prop(&props, "Ssid") else {
                        continue;
                    };
                    if ssid.is_empty() {
                        continue;
                    }
                    let signal = u8_prop(&props, "Strength");
                    let flags = u32_prop(&props, "Flags").unwrap_or(0);
                    let wpa = u32_prop(&props, "WpaFlags").unwrap_or(0);
                    let rsn = u32_prop(&props, "RsnFlags").unwrap_or(0);
                    let security = nm_security(flags, wpa, rsn);
                    let frequency = u32_prop(&props, "Frequency");
                    let bssid = string_prop(&props, "HwAddress");
                    let entry = Network {
                        saved: saved.iter().any(|(name, _, _)| name == &ssid),
                        signal,
                        security,
                        frequency,
                        bssid,
                        ssid: ssid.clone(),
                        ..Default::default()
                    };
                    let key = (ssid, entry.security.clone());
                    let replace = networks
                        .get(&key)
                        .map(|old| entry.signal.unwrap_or(0) > old.signal.unwrap_or(0))
                        .unwrap_or(true);
                    if replace {
                        networks.insert(key, entry);
                    }
                }
            }
        }
    }

    let mut current = String::new();
    if let Some(active) = active {
        if let Ok(active_props) = get_all(&conn, NM, active.as_str(), NM_ACTIVE) {
            snap.active_profile = path_prop(&active_props, "Connection").unwrap_or_default();
            let id = string_prop(&active_props, "Id").unwrap_or_default();
            if let Some(specific) = path_prop(&active_props, "SpecificObject") {
                if let Ok(ap_props) = get_all(&conn, NM, &specific, NM_AP) {
                    if let Some(ssid) = bytes_prop(&ap_props, "Ssid") {
                        if !ssid.is_empty() {
                            current = ssid.clone();
                            if let Some(signal) = u8_prop(&ap_props, "Strength") {
                                snap.signal = Some(signal);
                            }
                            let flags = u32_prop(&ap_props, "Flags").unwrap_or(0);
                            let wpa = u32_prop(&ap_props, "WpaFlags").unwrap_or(0);
                            let rsn = u32_prop(&ap_props, "RsnFlags").unwrap_or(0);
                            snap.security = nm_security(flags, wpa, rsn);
                        }
                    }
                }
            }
            if current.is_empty() {
                current = id;
            }
            if let Some(ip4) = path_prop(&active_props, "Ip4Config") {
                if let Ok(ip_props) = get_all(&conn, NM, &ip4, NM_IP4) {
                    snap.address = nm_address(&ip_props).unwrap_or_default();
                }
            }
        }
    }
    if !current.is_empty() {
        snap.ssid = current.clone();
        if snap.connected {
            snap.state = "connected".into();
        }
    }

    for (ssid, security, _) in saved {
        if networks.keys().any(|(name, _)| name == &ssid) {
            continue;
        }
        networks.insert(
            (ssid.clone(), security.clone()),
            Network {
                ssid,
                security,
                saved: true,
                ..Default::default()
            },
        );
    }
    if !snap.ssid.is_empty() && !networks.keys().any(|(ssid, _)| ssid == &snap.ssid) {
        networks.insert(
            (snap.ssid.clone(), snap.security.clone()),
            Network {
                ssid: snap.ssid.clone(),
                signal: snap.signal,
                security: snap.security.clone(),
                saved: snap.saved.contains(&snap.ssid),
                active: true,
                ..Default::default()
            },
        );
    }
    for network in networks.values_mut() {
        network.active = snap.connected && network.ssid == snap.ssid;
    }
    let mut networks: Vec<Network> = networks.into_values().collect();
    networks.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then_with(|| b.saved.cmp(&a.saved))
            .then_with(|| b.signal.unwrap_or(0).cmp(&a.signal.unwrap_or(0)))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    snap.networks = networks;
    if !snap.powered {
        snap.state = "off".into();
        snap.connected = false;
    }
    Ok(snap)
}

fn nm_saved(conn: &Connection) -> Result<Vec<(String, String, OwnedObjectPath)>, String> {
    let settings = proxy(
        conn,
        NM,
        NM_SETTINGS,
        "org.freedesktop.NetworkManager.Settings",
    )?;
    let paths: Vec<OwnedObjectPath> = settings
        .call("ListConnections", &())
        .map_err(|e| format!("Saved networks: {e}"))?;
    let mut out = Vec::new();
    for path in paths {
        let path_string = path.to_string();
        let conn_proxy = proxy(
            conn,
            NM,
            &path_string,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )?;
        let Ok(settings) = conn_proxy
            .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
        else {
            continue;
        };
        let Some(wireless) = settings.get("802-11-wireless") else {
            continue;
        };
        let Some(ssid) = bytes_prop(wireless, "ssid") else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        let security = match settings.get("802-11-wireless-security") {
            Some(sec) => {
                let key = string_prop(sec, "key-mgmt").unwrap_or_default();
                match key.as_str() {
                    "sae" => "WPA3",
                    "owe" => "OWE",
                    "wpa-eap" | "wpa-eap-suite-b-192" => "Saved Enterprise",
                    "wpa-psk" => "Saved WPA2/WPA3",
                    _ => "Saved",
                }
                .to_string()
            }
            None => "Open".into(),
        };
        out.push((ssid, security, path));
    }
    Ok(out)
}

pub fn connect(ssid: &str, security: &str, password: &str, hidden: bool) -> Result<(), String> {
    if ssid.is_empty() {
        return Err("Choose a network first".into());
    }
    if security.contains("Enterprise") {
        return Err(
            "Enterprise networks need a saved NetworkManager profile in this release".into(),
        );
    }
    if security == "WEP" {
        return Err("WEP is not supported; choose a WPA2 or WPA3 network".into());
    }
    if !matches!(security, "Open" | "OWE" | "Unknown" | "Saved") && !security.starts_with("Saved") {
        let valid = (8..=63).contains(&password.len())
            || (password.len() == 64 && password.chars().all(|c| c.is_ascii_hexdigit()));
        if !valid {
            return Err("A WPA password must be 8–63 characters or 64 hex digits".into());
        }
    }
    let Some(interface) = wireless_interface() else {
        return Err("No Wi-Fi hardware found".into());
    };
    match manager_for(&interface).0 {
        Manager::NetworkManager => nm_connect(&interface, ssid, security, password, hidden),
        Manager::WpaSupplicant => wpa_connect(&interface, ssid, security, password, hidden),
        manager @ (Manager::Iwd | Manager::ConnMan) => {
            if hidden {
                return Err(
                    "Hidden networks require an existing backend profile for this adapter".into(),
                );
            }
            native::connect(&interface, manager, ssid, password)
        }
        Manager::None => Err("No supported Wi-Fi manager owns this interface".into()),
    }
}

pub fn disconnect() -> Result<(), String> {
    let (interface, manager) = current()?;
    match manager {
        Manager::NetworkManager => {
            let conn = Connection::system().map_err(|e| e.to_string())?;
            let device = nm_device_path(&conn, &interface)?;
            let result = proxy(&conn, NM, device.as_str(), NM_DEVICE)?
                .call::<_, _, ()>("Disconnect", &())
                .map_err(|e| format!("Disconnect: {e}"));
            result
        }
        Manager::WpaSupplicant => wpa_run(&interface, &["disconnect"]).map(|_| ()),
        manager @ (Manager::Iwd | Manager::ConnMan) => {
            native::control(&interface, manager, "disconnect", None)
        }
        _ => Err(format!(
            "{} cannot be controlled in this release",
            manager.label()
        )),
    }
}

pub fn forget(ssid: &str) -> Result<(), String> {
    let (interface, manager) = current()?;
    match manager {
        Manager::NetworkManager => {
            let conn = Connection::system().map_err(|e| e.to_string())?;
            let saved = nm_saved(&conn)?;
            let matches: Vec<_> = saved
                .iter()
                .filter(|(name, _, path)| path.as_str() == ssid || name == ssid)
                .collect();
            if matches.len() > 1 {
                return Err("Choose the exact saved profile before forgetting it".into());
            }
            let Some((_, _, path)) = matches.first().copied() else {
                return Err("That network has no saved profile".into());
            };
            let settings = proxy(
                &conn,
                NM,
                path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )?;
            settings
                .call::<_, _, ()>("Delete", &())
                .map_err(|e| format!("Forget {ssid}: {e}"))
        }
        Manager::WpaSupplicant => wpa_forget(&interface, ssid),
        manager @ (Manager::Iwd | Manager::ConnMan) => {
            native::control(&interface, manager, "forget", Some(ssid))
        }
        _ => Err(format!(
            "{} cannot be controlled in this release",
            manager.label()
        )),
    }
}

pub fn set_power(on: bool) -> Result<(), String> {
    let (_interface, manager) = current()?;
    match manager {
        Manager::NetworkManager => {
            let conn = Connection::system().map_err(|e| e.to_string())?;
            let nm = proxy(&conn, NM, NM_PATH, NM)?;
            // NetworkManager.Enable(bool) controls *all* networking. Wi-Fi has
            // its own WirelessEnabled property; using Enable(false) here once
            // left every interface unmanaged and cut the machine off the
            // network. Never touch the global switch for the Wi-Fi radio.
            nm.set_property("WirelessEnabled", on)
                .map_err(|e| format!("Wi-Fi power: {e}"))
        }
        Manager::WpaSupplicant => {
            let helper = Path::new("/usr/libexec/nirux-wifi");
            privileged(
                helper,
                &["radio".into(), if on { "on" } else { "off" }.into()],
                None,
            )
            .map(|_| ())
        }
        manager @ (Manager::Iwd | Manager::ConnMan) => {
            native::control(&_interface, manager, if on { "on" } else { "off" }, None)
        }
        _ => Err(format!(
            "{} cannot be controlled in this release",
            manager.label()
        )),
    }
}

fn current() -> Result<(String, Manager), String> {
    let interface = wireless_interface().ok_or("No Wi-Fi hardware found")?;
    let manager = manager_for(&interface).0;
    if manager == Manager::None {
        return Err("No supported Wi-Fi manager owns this interface".into());
    }
    Ok((interface, manager))
}

fn ov_string(value: &str) -> OwnedValue {
    OwnedValue::try_from(Value::from(value.to_string())).expect("string value")
}

fn ov_bytes(value: &[u8]) -> OwnedValue {
    OwnedValue::try_from(Value::from(value.to_vec())).expect("byte array value")
}

pub fn connect_profile(id: &str) -> Result<(), String> {
    let (interface, manager) = current()?;
    if manager != Manager::NetworkManager {
        return Err("That profile belongs to another backend".into());
    }
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let profile = nm_saved(&conn)?
        .into_iter()
        .find(|(_, _, path)| path.as_str() == id)
        .ok_or("Saved profile no longer exists; refresh the list")?;
    let device = nm_device_path(&conn, &interface)?;
    proxy(&conn, NM, NM_PATH, NM)?
        .call::<_, _, OwnedObjectPath>(
            "ActivateConnection",
            &(
                profile.2.clone(),
                device.clone(),
                OwnedObjectPath::try_from("/").unwrap(),
            ),
        )
        .map_err(|e| format!("Activate saved profile: {e}"))?;
    wait_nm_activation(&conn, &device, &profile.2)
}

fn wait_nm_activation(
    conn: &Connection,
    device: &OwnedObjectPath,
    profile: &OwnedObjectPath,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(35);
    while std::time::Instant::now() < deadline {
        let props = get_all(conn, NM, device.as_str(), NM_DEVICE)?;
        let state = u32_prop(&props, "State").unwrap_or(0);
        if state == 100 {
            if let Some(active) = path_prop(&props, "ActiveConnection") {
                let active = get_all(conn, NM, &active, NM_ACTIVE)?;
                if path_prop(&active, "Connection").as_deref() == Some(profile.as_str()) {
                    return Ok(());
                }
            }
        }
        if state == 120 {
            return Err("Connection failed; check authentication and address configuration".into());
        }
        std::thread::sleep(Duration::from_millis(350));
    }
    Err("Connection has not completed; check the backend state before retrying".into())
}

fn nm_connect(
    interface: &str,
    ssid: &str,
    security: &str,
    password: &str,
    hidden: bool,
) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let device = nm_device_path(&conn, interface)?;
    let nm = proxy(&conn, NM, NM_PATH, NM)?;
    if password.is_empty() && !hidden {
        if let Ok(saved) = nm_saved(&conn) {
            let matches: Vec<_> = saved
                .iter()
                .filter(|(name, kind, _)| {
                    name == ssid
                        && (security == "Saved" || kind.contains(security) || kind == security)
                })
                .collect();
            if matches.len() > 1 {
                return Err(
                    "Several saved profiles match; choose an exact Saved profile row".into(),
                );
            }
            if let Some((_, _, path)) = matches.first().copied() {
                nm.call::<_, _, OwnedObjectPath>(
                    "ActivateConnection",
                    &(
                        path.clone(),
                        device.clone(),
                        OwnedObjectPath::try_from("/").unwrap(),
                    ),
                )
                .map_err(|e| format!("Connect {ssid}: {e}"))?;
                return wait_nm_activation(&conn, &device, path);
            }
        }
        if !security.starts_with("Saved") && security != "Open" && security != "OWE" {
            return Err("This network needs a password".into());
        }
    }
    let mut connection: HashMap<String, OwnedValue> = HashMap::new();
    connection.insert("id".into(), ov_string(ssid));
    connection.insert("type".into(), ov_string("802-11-wireless"));
    connection.insert("uuid".into(), ov_string(&uuid()));
    let mut wireless: HashMap<String, OwnedValue> = HashMap::new();
    wireless.insert("ssid".into(), ov_bytes(ssid.as_bytes()));
    wireless.insert("mode".into(), ov_string("infrastructure"));
    if hidden {
        wireless.insert("hidden".into(), OwnedValue::from(true));
    }
    let mut settings: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
    if !matches!(security, "Open" | "OWE") {
        wireless.insert("security".into(), ov_string("802-11-wireless-security"));
        let mut security_settings: HashMap<String, OwnedValue> = HashMap::new();
        let key_mgmt = if security == "WPA3" { "sae" } else { "wpa-psk" };
        security_settings.insert("key-mgmt".into(), ov_string(key_mgmt));
        security_settings.insert("psk".into(), ov_string(password));
        settings.insert("802-11-wireless-security".into(), security_settings);
    } else if security == "OWE" {
        wireless.insert("security".into(), ov_string("802-11-wireless-security"));
        let mut security_settings: HashMap<String, OwnedValue> = HashMap::new();
        security_settings.insert("key-mgmt".into(), ov_string("owe"));
        settings.insert("802-11-wireless-security".into(), security_settings);
    }
    settings.insert("connection".into(), connection);
    settings.insert("802-11-wireless".into(), wireless);
    let mut ipv4 = HashMap::new();
    ipv4.insert("method".into(), ov_string("auto"));
    settings.insert("ipv4".into(), ipv4);
    let mut ipv6 = HashMap::new();
    ipv6.insert("method".into(), ov_string("auto"));
    settings.insert("ipv6".into(), ipv6);
    let (profile, _) = nm
        .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
            "AddAndActivateConnection",
            &(
                settings,
                device.clone(),
                OwnedObjectPath::try_from("/").unwrap(),
            ),
        )
        .map_err(|e| format!("Connect {ssid}: {e}"))?;
    wait_nm_activation(&conn, &device, &profile)
}

fn wpa_snapshot(interface: &str, scan: bool) -> Result<Snapshot, String> {
    let link = iw_link(interface);
    let mut snap = Snapshot {
        present: true,
        interface: interface.into(),
        manager: "wpa_supplicant".into(),
        powered: !rfkill_state().0 && !rfkill_state().1,
        connected: link.connected,
        ssid: link.ssid.clone(),
        signal: link.signal,
        state: if link.connected {
            "connected"
        } else {
            "disconnected"
        }
        .into(),
        detail: "Standalone wpa_supplicant".into(),
        ..Default::default()
    };
    let status = wpa_run(interface, &["status"]);
    if let Ok(output) = &status {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("wpa_state=") {
                if value == "COMPLETED" {
                    snap.connected = true;
                    snap.state = "connected".into();
                }
            } else if let Some(value) = line.strip_prefix("ssid=") {
                if !value.is_empty() {
                    snap.ssid = value.to_string();
                }
            } else if let Some(value) = line.strip_prefix("ip_address=") {
                snap.address = value.to_string();
            }
        }
    } else if let Some(error) = status.err() {
        snap.error = error;
    }
    if scan {
        match wpa_scan(interface) {
            Ok(mut networks) => {
                for network in &mut networks {
                    if snap.connected && network.ssid == snap.ssid {
                        network.active = true;
                        snap.signal = network.signal;
                        snap.security = network.security.clone();
                    }
                }
                networks.sort_by(|a, b| {
                    b.active
                        .cmp(&a.active)
                        .then_with(|| b.signal.unwrap_or(0).cmp(&a.signal.unwrap_or(0)))
                        .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
                });
                snap.networks = networks;
            }
            Err(error) => snap.error = error,
        }
    }
    if let Ok(saved) = wpa_saved(interface) {
        for (ssid, _) in &saved {
            if !snap.saved.contains(ssid) {
                snap.saved.push(ssid.clone());
            }
            if !snap.networks.iter().any(|network| &network.ssid == ssid) {
                snap.networks.push(Network {
                    ssid: ssid.clone(),
                    security: "Saved".into(),
                    saved: true,
                    ..Default::default()
                });
            }
        }
    }
    Ok(snap)
}

fn wpa_scan(interface: &str) -> Result<Vec<Network>, String> {
    wpa_run(interface, &["scan"])?;
    std::thread::sleep(Duration::from_millis(1200));
    let output = wpa_run(interface, &["scan_results"])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let saved: HashSet<String> = wpa_saved(interface)
        .map(|items| items.into_iter().map(|(ssid, _)| ssid).collect())
        .unwrap_or_default();
    let mut networks: HashMap<(String, String), Network> = HashMap::new();
    for line in text.lines().skip(1) {
        let mut fields = line.splitn(5, '\t');
        let bssid = fields.next().unwrap_or("").trim();
        let frequency = fields.next().and_then(|v| v.trim().parse::<u32>().ok());
        let signal = fields.next().and_then(|v| v.trim().parse::<i32>().ok());
        let flags = fields.next().unwrap_or("").trim();
        let ssid = fields.next().unwrap_or("").trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let security = wpa_security(flags);
        let signal = signal.map(dbm_percent);
        let entry = Network {
            saved: saved.contains(&ssid),
            ssid: ssid.clone(),
            signal,
            security: security.clone(),
            frequency,
            bssid: (!bssid.is_empty()).then(|| bssid.to_string()),
            ..Default::default()
        };
        let key = (ssid, security);
        let replace = networks
            .get(&key)
            .map(|old| entry.signal.unwrap_or(0) > old.signal.unwrap_or(0))
            .unwrap_or(true);
        if replace {
            networks.insert(key, entry);
        }
    }
    Ok(networks.into_values().collect())
}

fn wpa_saved(interface: &str) -> Result<Vec<(String, usize)>, String> {
    let output = wpa_run(interface, &["list_networks"])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 2 {
            continue;
        }
        let Ok(id) = fields[0].trim().parse::<usize>() else {
            continue;
        };
        let ssid = fields[1].trim().to_string();
        if !ssid.is_empty() {
            out.push((ssid, id));
        }
    }
    Ok(out)
}

fn wpa_connect(
    interface: &str,
    ssid: &str,
    security: &str,
    password: &str,
    hidden: bool,
) -> Result<(), String> {
    let ssid_q = wpa_quote(ssid)?;
    wpa_quote(password)?;
    if security.contains("Enterprise") {
        return Err("Enterprise networks need a saved wpa_supplicant profile".into());
    }
    if password.is_empty() {
        if let Ok(saved) = wpa_saved(interface) {
            if let Some((_, id)) = saved.into_iter().find(|(name, _)| name == ssid) {
                wpa_run_stdin(interface, &format!("select_network {id}\nquit\n"))?;
                return wait_wpa_activation(interface, ssid);
            }
        }
        if !matches!(security, "Open" | "OWE") {
            return Err("This network needs a password".into());
        }
    }
    let previous = wpa_run(interface, &["status"]).ok().and_then(|out| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|line| {
                line.strip_prefix("id=")
                    .and_then(|id| id.parse::<usize>().ok())
            })
    });
    let output = wpa_run(interface, &["add_network"])?;
    let id = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<usize>().ok())
        .ok_or("wpa_supplicant did not allocate a network")?;
    let mut commands = format!("set_network {id} ssid \"{ssid_q}\"\n");
    if hidden {
        commands.push_str(&format!("set_network {id} scan_ssid 1\n"));
    }
    match security {
        "Open" | "Unknown" | "Saved" => {
            commands.push_str(&format!("set_network {id} key_mgmt NONE\n"));
        }
        "OWE" => {
            commands.push_str(&format!("set_network {id} key_mgmt OWE\n"));
        }
        "WPA3" => {
            let password = wpa_quote(password)?;
            commands.push_str(&format!("set_network {id} key_mgmt SAE\n"));
            commands.push_str(&format!("set_network {id} sae_password \"{password}\"\n"));
        }
        "WPA2/WPA3" => {
            let password = wpa_quote(password)?;
            commands.push_str(&format!("set_network {id} key_mgmt WPA-PSK SAE\n"));
            commands.push_str(&format!("set_network {id} psk \"{password}\"\n"));
            commands.push_str(&format!("set_network {id} sae_password \"{password}\"\n"));
        }
        _ => {
            let password = wpa_quote(password)?;
            commands.push_str(&format!("set_network {id} key_mgmt WPA-PSK\n"));
            commands.push_str(&format!("set_network {id} psk \"{password}\"\n"));
        }
    }
    commands.push_str(&format!("enable_network {id}\nselect_network {id}\nquit\n"));
    let result = wpa_run_stdin(interface, &commands)
        .and_then(|_| wait_wpa_activation(interface, ssid))
        .and_then(|_| wpa_run_stdin(interface, "save_config\nquit\n").map(|_| ()));
    if let Err(error) = result {
        let mut restore = format!("remove_network {id}\n");
        if let Some(previous) = previous {
            restore.push_str(&format!("select_network {previous}\n"));
        }
        restore.push_str("quit\n");
        let _ = wpa_run_stdin(interface, &restore);
        return Err(error);
    }
    Ok(())
}

fn wait_wpa_activation(interface: &str, ssid: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(35);
    while std::time::Instant::now() < deadline {
        let output = wpa_run(interface, &["status"])?;
        let text = String::from_utf8_lossy(&output.stdout);
        let completed = text.lines().any(|line| line == "wpa_state=COMPLETED");
        let correct = text
            .lines()
            .any(|line| line.strip_prefix("ssid=") == Some(ssid));
        if completed && correct {
            let addressed = Command::new("ip")
                .args(["-j", "address", "show", "dev", interface])
                .output()
                .ok()
                .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
                .and_then(|value| value.as_array().cloned())
                .is_some_and(|devices| {
                    devices.iter().any(|device| {
                        device
                            .get("addr_info")
                            .and_then(|a| a.as_array())
                            .is_some_and(|addresses| {
                                addresses.iter().any(|address| {
                                    address.get("scope").and_then(|v| v.as_str()) == Some("global")
                                })
                            })
                    })
                });
            if addressed {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(
        "Wi-Fi authentication or address configuration did not complete; previous profile retained"
            .into(),
    )
}

fn wpa_forget(interface: &str, ssid: &str) -> Result<(), String> {
    let saved = wpa_saved(interface)?;
    let (_, id) = saved
        .into_iter()
        .find(|(name, _)| name == ssid)
        .ok_or("That network has no saved profile")?;
    wpa_run_stdin(
        interface,
        &format!("remove_network {id}\nsave_config\nquit\n"),
    )
    .map(|_| ())
}

fn wpa_quote(value: &str) -> Result<String, String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err("That name contains a character Wi-Fi cannot use".into());
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn wpa_run(interface: &str, args: &[&str]) -> Result<Output, String> {
    let program = wpa_cli_path().ok_or("wpa_cli is not installed")?;
    let mut full = vec!["-i".to_string(), interface.to_string()];
    full.extend(args.iter().map(|value| value.to_string()));
    privileged(&program, &full, None)
}

fn wpa_run_stdin(interface: &str, input: &str) -> Result<Output, String> {
    let program = wpa_cli_path().ok_or("wpa_cli is not installed")?;
    let full = vec!["-i".to_string(), interface.to_string()];
    privileged(&program, &full, Some(input))
}

fn privileged(program: &Path, args: &[String], input: Option<&str>) -> Result<Output, String> {
    let helper = Path::new("/usr/libexec/nirux-wifi");
    if !helper.is_file() {
        return Err("Standalone Wi-Fi permissions are missing. Run nirux update to install the restricted Wi-Fi helper.".into());
    }
    let _ = program; // Never elevate an arbitrary CLI, even when it is system-owned.
    let program = helper;
    let sudo = command_path("sudo").ok_or("sudo is not installed")?;
    let mut command = Command::new(sudo);
    command.arg("-n").arg(program).args(args);
    let output = run_with_input(&mut command, input)?;
    if output.status.success() {
        if String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim().starts_with("FAIL"))
        {
            return Err("The Wi-Fi backend rejected the operation".into());
        }
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if stderr.contains("password") || stderr.contains("a password is required") {
        if let Some(pkexec) = command_path("pkexec") {
            if std::env::var_os("DISPLAY").is_some() {
                let mut command = Command::new(pkexec);
                command.arg(program).args(args);
                return run_with_input(&mut command, input);
            }
        }
    }
    Err(if output.stderr.is_empty() {
        format!("{} failed with status {}", program.display(), output.status)
    } else {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    })
}

fn run_with_input(command: &mut Command, input: Option<&str>) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|e| {
        format!(
            "cannot start {}: {e}",
            command.get_program().to_string_lossy()
        )
    })?;
    if let Some(input) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .map_err(|e| format!("cannot send Wi-Fi control input: {e}"))?;
        }
    }
    child
        .wait_with_output()
        .map_err(|e| format!("Wi-Fi control process: {e}"))
}

fn wpa_security(flags: &str) -> String {
    let upper = flags.to_ascii_uppercase();
    if upper.contains("EAP") {
        return "Enterprise".into();
    }
    if upper.contains("SAE") && upper.contains("PSK") {
        return "WPA2/WPA3".into();
    }
    if upper.contains("SAE") {
        return "WPA3".into();
    }
    if upper.contains("WPA2") {
        return "WPA2".into();
    }
    if upper.contains("WPA") {
        return "WPA".into();
    }
    if upper.contains("WEP") {
        return "WEP".into();
    }
    if upper.contains("OWE") {
        return "OWE".into();
    }
    "Open".into()
}

fn nm_security(flags: u32, wpa: u32, rsn: u32) -> String {
    let enterprise = wpa & AP_KEY_EAP != 0 || rsn & AP_KEY_EAP != 0;
    let psk = wpa & AP_KEY_PSK != 0 || rsn & AP_KEY_PSK != 0;
    let sae = rsn & AP_KEY_SAE != 0;
    let owe = rsn & AP_KEY_OWE != 0;
    if enterprise {
        return "Enterprise".into();
    }
    if owe {
        return "OWE".into();
    }
    if sae && psk {
        return "WPA2/WPA3".into();
    }
    if sae {
        return "WPA3".into();
    }
    if psk {
        return if rsn != 0 { "WPA2" } else { "WPA" }.into();
    }
    if flags & AP_PRIVACY != 0 {
        return "WEP".into();
    }
    "Open".into()
}

fn nm_address(props: &HashMap<String, OwnedValue>) -> Option<String> {
    let value = props.get("AddressData")?.try_clone().ok()?;
    let addresses: Vec<HashMap<String, OwnedValue>> = value.try_into().ok()?;
    let first = addresses.first()?;
    let address = string_prop(first, "address")?;
    let prefix = u32_prop(first, "prefix").unwrap_or(0);
    Some(format!("{address}/{prefix}"))
}

fn nm_state_name(state: u32) -> &'static str {
    match state {
        0 => "unknown",
        10 => "unmanaged",
        20 => "unavailable",
        30 => "disconnected",
        40 => "preparing",
        50 => "configuring",
        60 => "needs authentication",
        70 => "obtaining address",
        80 => "checking address",
        90 => "secondary connection",
        100 => "connected",
        110 => "deactivating",
        120 => "failed",
        _ => "unknown",
    }
}

fn wireless_interface() -> Option<String> {
    let entries = fs::read_dir("/sys/class/net").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("p2p-dev-") {
            continue;
        }
        if entry.path().join("wireless").exists() {
            return Some(name);
        }
    }
    None
}

fn rfkill_state() -> (bool, bool) {
    let mut hard = false;
    let mut soft = false;
    let Ok(entries) = fs::read_dir("/sys/class/rfkill") else {
        return (false, false);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() != "wlan" {
            continue;
        }
        hard |= fs::read_to_string(path.join("hard"))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        soft |= fs::read_to_string(path.join("soft"))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
    }
    (hard, soft)
}

struct LinkState {
    connected: bool,
    ssid: String,
    signal: Option<u8>,
}

fn iw_link(interface: &str) -> LinkState {
    let Some(iw) = command_path("iw") else {
        return LinkState {
            connected: false,
            ssid: String::new(),
            signal: None,
        };
    };
    let Ok(output) = Command::new(iw).args(["dev", interface, "link"]).output() else {
        return LinkState {
            connected: false,
            ssid: String::new(),
            signal: None,
        };
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut ssid = String::new();
    let mut signal = None;
    let mut connected = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("SSID: ") {
            ssid = value.to_string();
            connected = true;
        } else if let Some(value) = line.strip_prefix("signal: ") {
            signal = value
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<i32>().ok())
                .map(dbm_percent);
        } else if line.contains("Not connected") {
            connected = false;
        }
    }
    LinkState {
        connected,
        ssid,
        signal,
    }
}

fn dbm_percent(dbm: i32) -> u8 {
    ((dbm + 90) * 100 / 60).clamp(0, 100) as u8
}

fn command_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for dir in ["/sbin", "/usr/sbin", "/bin", "/usr/bin"] {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn wpa_cli_path() -> Option<PathBuf> {
    if let Some(path) = command_path("wpa_cli") {
        return Some(path);
    }
    for path in ["/sbin/wpa_cli", "/usr/sbin/wpa_cli", "/usr/bin/wpa_cli"] {
        let path = Path::new(path);
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn wpa_process_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let comm = fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        if comm.trim() == "wpa_supplicant" {
            return true;
        }
    }
    false
}

fn proxy<'a>(
    conn: &'a Connection,
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, destination, path, interface).map_err(|e| e.to_string())
}

fn get_all(
    conn: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
) -> Result<HashMap<String, OwnedValue>, String> {
    let props = proxy(conn, destination, path, "org.freedesktop.DBus.Properties")?;
    props
        .call("GetAll", &(interface,))
        .map_err(|e| format!("read {interface}: {e}"))
}

fn string_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    String::try_from(props.get(name)?.try_clone().ok()?).ok()
}

fn bool_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<bool> {
    bool::try_from(props.get(name)?).ok()
}

fn u8_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<u8> {
    u8::try_from(props.get(name)?).ok()
}

fn u32_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<u32> {
    u32::try_from(props.get(name)?).ok()
}

fn bytes_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    let value = props.get(name)?.try_clone().ok()?;
    let bytes: Vec<u8> = value.try_into().ok()?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

fn path_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    let path = OwnedObjectPath::try_from(props.get(name)?.try_clone().ok()?).ok()?;
    let value = path.to_string();
    (value != "/").then_some(value)
}

fn uuid() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = file.read_exact(&mut bytes);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nm_security_labels_common_aps() {
        assert_eq!(nm_security(0, 0, 0), "Open");
        assert_eq!(nm_security(1, 0, 0), "WEP");
        assert_eq!(nm_security(0, AP_KEY_PSK, AP_KEY_PSK), "WPA2");
        assert_eq!(
            nm_security(0, AP_KEY_PSK, AP_KEY_PSK | AP_KEY_SAE),
            "WPA2/WPA3"
        );
        assert_eq!(nm_security(0, 0, AP_KEY_SAE), "WPA3");
        assert_eq!(nm_security(0, 0, AP_KEY_EAP), "Enterprise");
    }

    #[test]
    fn wpa_security_labels_common_aps() {
        assert_eq!(wpa_security("[ESS]"), "Open");
        assert_eq!(wpa_security("[WPA2-PSK-CCMP][ESS]"), "WPA2");
        assert_eq!(wpa_security("[WPA2-SAE-CCMP][ESS]"), "WPA3");
        assert_eq!(wpa_security("[WPA2-PSK+SAE-CCMP][ESS]"), "WPA2/WPA3");
        assert_eq!(wpa_security("[WPA2-EAP-CCMP][ESS]"), "Enterprise");
    }

    #[test]
    fn secrets_with_quotes_and_swedish_are_safe_for_wpa_cli() {
        assert_eq!(wpa_quote("räksmörgås").unwrap(), "räksmörgås");
        assert_eq!(wpa_quote("a\"b\\c").unwrap(), "a\\\"b\\\\c");
        assert!(wpa_quote("a\nb").is_err());
    }

    #[test]
    fn signal_percent_is_bounded() {
        assert_eq!(dbm_percent(-30), 100);
        assert_eq!(dbm_percent(-120), 0);
    }

    #[test]
    fn saved_and_enterprise_networks_do_not_prompt_for_a_new_secret() {
        let saved = Network {
            security: "Saved".into(),
            ..Default::default()
        };
        assert!(!saved.needs_secret());
        let enterprise = Network {
            security: "Enterprise".into(),
            ..Default::default()
        };
        assert!(!enterprise.needs_secret());
        assert!(!enterprise.can_connect());
        let wpa = Network {
            security: "WPA2".into(),
            ..Default::default()
        };
        assert!(wpa.needs_secret());
        assert!(wpa.can_connect());
    }

    #[test]
    fn uuid_has_rfc4122_version_and_variant_bits() {
        let value = uuid();
        assert_eq!(value.len(), 36);
        assert_eq!(&value[14..15], "4");
        assert!(matches!(&value[19..20], "8" | "9" | "a" | "b"));
    }
}
