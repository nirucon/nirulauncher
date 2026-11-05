//! Native adapters for standalone iwd and ConnMan. No manager is started or replaced.
use super::*;

type Properties = HashMap<String, OwnedValue>;
type Interfaces = HashMap<String, Properties>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;

const IWD: &str = "net.connman.iwd";
const CM: &str = "net.connman";

fn strings(props: &Properties, name: &str) -> Vec<String> {
    props
        .get(name)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default()
}

fn iwd_objects(conn: &Connection) -> Result<Objects, String> {
    proxy(conn, IWD, "/", "org.freedesktop.DBus.ObjectManager")?
        .call("GetManagedObjects", &())
        .map_err(|e| format!("iwd discovery: {e}"))
}

fn iwd_device(objects: &Objects, interface: &str) -> Result<OwnedObjectPath, String> {
    objects
        .iter()
        .find_map(|(path, interfaces)| {
            interfaces.get("net.connman.iwd.Device").and_then(|props| {
                (string_prop(props, "Name").as_deref() == Some(interface)).then(|| path.clone())
            })
        })
        .ok_or_else(|| format!("iwd does not own {interface}"))
}

fn technologies(conn: &Connection) -> Result<Vec<(OwnedObjectPath, Properties)>, String> {
    proxy(conn, CM, "/", "net.connman.Manager")?
        .call("GetTechnologies", &())
        .map_err(|e| format!("ConnMan technologies: {e}"))
}

fn services(conn: &Connection) -> Result<Vec<(OwnedObjectPath, Properties)>, String> {
    proxy(conn, CM, "/", "net.connman.Manager")?
        .call("GetServices", &())
        .map_err(|e| format!("ConnMan services: {e}"))
}

fn security(kind: &str) -> String {
    match kind {
        "open" | "none" => "Open",
        "psk" => "WPA2/WPA3",
        "sae" => "WPA3",
        "8021x" | "ieee8021x" => "Enterprise",
        "wep" => "WEP",
        "owe" => "OWE",
        _ => "Unknown",
    }
    .into()
}

pub(super) fn owns(interface: &str, manager: Manager) -> bool {
    let Ok(conn) = Connection::system() else {
        return false;
    };
    match manager {
        Manager::Iwd => iwd_objects(&conn)
            .and_then(|objects| iwd_device(&objects, interface))
            .is_ok(),
        Manager::ConnMan => services(&conn)
            .map(|items| {
                items.iter().any(|(_, props)| {
                    // ConnMan's Ethernet dictionary identifies the actual Wi-Fi device.
                    props
                        .get("Ethernet")
                        .and_then(|v| v.try_clone().ok())
                        .and_then(|v| Properties::try_from(v).ok())
                        .and_then(|v| string_prop(&v, "Interface"))
                        .as_deref()
                        == Some(interface)
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

pub(super) fn snapshot(interface: &str, manager: Manager, scan: bool) -> Result<Snapshot, String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let mut snap = Snapshot {
        present: true,
        interface: interface.into(),
        manager: manager.label().into(),
        powered: true,
        ..Default::default()
    };
    match manager {
        Manager::Iwd => {
            let mut objects = iwd_objects(&conn)?;
            let device = iwd_device(&objects, interface)?;
            if scan {
                if let Err(error) = proxy(&conn, IWD, device.as_str(), "net.connman.iwd.Station")?
                    .call::<_, _, ()>("Scan", &())
                {
                    snap.error = format!("Scan: {error}");
                } else {
                    std::thread::sleep(Duration::from_millis(1000));
                }
                objects = iwd_objects(&conn)?;
            }
            if let Some(props) = objects
                .get(&device)
                .and_then(|v| v.get("net.connman.iwd.Device"))
            {
                snap.powered = bool_prop(props, "Powered").unwrap_or(false);
            }
            let ordered: Vec<(OwnedObjectPath, i16)> =
                proxy(&conn, IWD, device.as_str(), "net.connman.iwd.Station")?
                    .call("GetOrderedNetworks", &())
                    .unwrap_or_default();
            for (path, interfaces) in &objects {
                let Some(props) = interfaces.get("net.connman.iwd.Network") else {
                    continue;
                };
                if !path.as_str().starts_with(&format!("{}/", device.as_str())) {
                    continue;
                }
                let Some(ssid) = string_prop(props, "Name") else {
                    continue;
                };
                let active = bool_prop(props, "Connected").unwrap_or(false);
                let signal = ordered
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, dbm)| dbm_percent(i32::from(*dbm) / 100));
                let saved = path_prop(props, "KnownNetwork").is_some();
                let kind = security(&string_prop(props, "Type").unwrap_or_default());
                if active {
                    snap.connected = true;
                    snap.ssid = ssid.clone();
                    snap.signal = signal;
                    snap.security = kind.clone();
                }
                if saved {
                    snap.saved.push(ssid.clone());
                }
                snap.networks.push(Network {
                    ssid,
                    signal,
                    security: kind,
                    saved,
                    active,
                    ..Default::default()
                });
            }
            snap.detail = "Standalone iwd; IP/DNS remain with the existing configured owner".into();
        }
        Manager::ConnMan => {
            for (path, props) in technologies(&conn)? {
                if string_prop(&props, "Type").as_deref() != Some("wifi") {
                    continue;
                }
                snap.powered = bool_prop(&props, "Powered").unwrap_or(false);
                if scan {
                    if let Err(error) = proxy(&conn, CM, path.as_str(), "net.connman.Technology")?
                        .call::<_, _, ()>("Scan", &())
                    {
                        snap.error = format!("Scan: {error}");
                    }
                }
            }
            for (_, props) in services(&conn)? {
                if string_prop(&props, "Type").as_deref() != Some("wifi") {
                    continue;
                }
                let Some(ssid) = string_prop(&props, "Name") else {
                    continue;
                };
                let state = string_prop(&props, "State").unwrap_or_default();
                let active = matches!(state.as_str(), "ready" | "online");
                let saved = bool_prop(&props, "Favorite").unwrap_or(false);
                let signal = u8_prop(&props, "Strength");
                let kind = strings(&props, "Security")
                    .first()
                    .map(|s| security(s))
                    .unwrap_or_else(|| "Unknown".into());
                if active {
                    snap.connected = true;
                    snap.ssid = ssid.clone();
                    snap.security = kind.clone();
                    snap.signal = signal;
                }
                if saved {
                    snap.saved.push(ssid.clone());
                }
                snap.networks.push(Network {
                    ssid,
                    security: kind,
                    signal,
                    active,
                    saved,
                    ..Default::default()
                });
            }
            snap.detail = "ConnMan manages the connection and IP/DNS".into();
        }
        _ => return Err("not a native standalone backend".into()),
    }
    snap.state = if !snap.powered {
        "off"
    } else if snap.connected {
        "connected"
    } else {
        "disconnected"
    }
    .into();
    snap.networks.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
    Ok(snap)
}

struct IwdAgent {
    network: OwnedObjectPath,
    password: String,
}
#[zbus::interface(name = "net.connman.iwd.Agent")]
impl IwdAgent {
    fn request_passphrase(&self, network: OwnedObjectPath) -> zbus::fdo::Result<String> {
        if network != self.network || self.password.is_empty() {
            return Err(zbus::fdo::Error::AccessDenied(
                "No secret for this network".into(),
            ));
        }
        Ok(self.password.clone())
    }
    fn cancel(&self, _reason: &str) {}
    fn release(&self) {}
}

struct ConnManAgent {
    service: OwnedObjectPath,
    password: String,
}
#[zbus::interface(name = "net.connman.Agent")]
impl ConnManAgent {
    fn request_input(
        &self,
        service: OwnedObjectPath,
        fields: Properties,
    ) -> zbus::fdo::Result<Properties> {
        if service != self.service || self.password.is_empty() || !fields.contains_key("Passphrase")
        {
            return Err(zbus::fdo::Error::AccessDenied(
                "Use an existing authenticated profile for this method".into(),
            ));
        }
        let mut reply = HashMap::new();
        reply.insert("Passphrase".into(), ov_string(&self.password));
        Ok(reply)
    }
    fn report_error(&self, _service: OwnedObjectPath, _error: &str) {}
    fn cancel(&self) {}
    fn release(&self) {}
}

pub(super) fn connect(
    interface: &str,
    manager: Manager,
    ssid: &str,
    password: &str,
) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let agent = format!("/org/niru/WifiAgent/{}", std::process::id());
    let agent_path = OwnedObjectPath::try_from(agent.as_str()).map_err(|e| e.to_string())?;
    match manager {
        Manager::Iwd => {
            let objects = iwd_objects(&conn)?;
            let device = iwd_device(&objects, interface)?;
            let network = objects
                .iter()
                .find_map(|(path, interfaces)| {
                    interfaces
                        .get("net.connman.iwd.Network")
                        .filter(|props| {
                            path.as_str().starts_with(&format!("{}/", device.as_str()))
                                && string_prop(props, "Name").as_deref() == Some(ssid)
                        })
                        .map(|_| path.clone())
                })
                .ok_or("network not found; refresh the list")?;
            if !password.is_empty() {
                conn.object_server()
                    .at(
                        agent.as_str(),
                        IwdAgent {
                            network: network.clone(),
                            password: password.into(),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                proxy(
                    &conn,
                    IWD,
                    "/net/connman/iwd",
                    "net.connman.iwd.AgentManager",
                )?
                .call::<_, _, ()>("RegisterAgent", &(agent_path.clone(),))
                .map_err(|e| e.to_string())?;
            }
            let result = proxy(&conn, IWD, network.as_str(), "net.connman.iwd.Network")?
                .call::<_, _, ()>("Connect", &())
                .map_err(|e| format!("iwd connect: {e}"));
            if !password.is_empty() {
                let _ = proxy(
                    &conn,
                    IWD,
                    "/net/connman/iwd",
                    "net.connman.iwd.AgentManager",
                )?
                .call::<_, _, ()>("UnregisterAgent", &(agent_path,));
            }
            result
        }
        Manager::ConnMan => {
            let service = services(&conn)?
                .into_iter()
                .find(|(_, props)| {
                    string_prop(props, "Type").as_deref() == Some("wifi")
                        && string_prop(props, "Name").as_deref() == Some(ssid)
                })
                .map(|(path, _)| path)
                .ok_or("network not found; refresh the list")?;
            if !password.is_empty() {
                conn.object_server()
                    .at(
                        agent.as_str(),
                        ConnManAgent {
                            service: service.clone(),
                            password: password.into(),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                proxy(&conn, CM, "/", "net.connman.Manager")?
                    .call::<_, _, ()>("RegisterAgent", &(agent_path.clone(),))
                    .map_err(|e| e.to_string())?;
            }
            let result = proxy(&conn, CM, service.as_str(), "net.connman.Service")?
                .call::<_, _, ()>("Connect", &())
                .map_err(|e| format!("ConnMan connect: {e}"));
            if !password.is_empty() {
                let _ = proxy(&conn, CM, "/", "net.connman.Manager")?
                    .call::<_, _, ()>("UnregisterAgent", &(agent_path,));
            }
            result
        }
        _ => Err("not a native standalone backend".into()),
    }
}

pub(super) fn control(
    interface: &str,
    manager: Manager,
    action: &str,
    ssid: Option<&str>,
) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    match manager {
        Manager::Iwd => {
            let objects = iwd_objects(&conn)?;
            let device = iwd_device(&objects, interface)?;
            match action {
                "on" | "off" => proxy(&conn, IWD, device.as_str(), "net.connman.iwd.Device")?
                    .set_property("Powered", action == "on")
                    .map_err(|e| e.to_string()),
                "disconnect" => proxy(&conn, IWD, device.as_str(), "net.connman.iwd.Station")?
                    .call::<_, _, ()>("Disconnect", &())
                    .map_err(|e| e.to_string()),
                "forget" => {
                    let known = objects
                        .iter()
                        .find_map(|(path, interfaces)| {
                            interfaces
                                .get("net.connman.iwd.KnownNetwork")
                                .filter(|props| string_prop(props, "Name").as_deref() == ssid)
                                .map(|_| path.clone())
                        })
                        .ok_or("saved network no longer exists")?;
                    let operation =
                        proxy(&conn, IWD, known.as_str(), "net.connman.iwd.KnownNetwork")?
                            .call::<_, _, ()>("Forget", &())
                            .map_err(|e| e.to_string());
                    operation
                }
                _ => Err("unsupported iwd operation".into()),
            }
        }
        Manager::ConnMan => {
            if matches!(action, "on" | "off") {
                let path = technologies(&conn)?
                    .into_iter()
                    .find(|(_, props)| string_prop(props, "Type").as_deref() == Some("wifi"))
                    .map(|(path, _)| path)
                    .ok_or("no ConnMan Wi-Fi technology")?;
                let operation = proxy(&conn, CM, path.as_str(), "net.connman.Technology")?
                    .call::<_, _, ()>(
                        "SetProperty",
                        &("Powered", OwnedValue::from(action == "on")),
                    )
                    .map_err(|e| e.to_string());
                operation
            } else {
                let path = services(&conn)?
                    .into_iter()
                    .find(|(_, props)| {
                        string_prop(props, "Type").as_deref() == Some("wifi")
                            && if action == "forget" {
                                string_prop(props, "Name").as_deref() == ssid
                            } else {
                                matches!(
                                    string_prop(props, "State").as_deref(),
                                    Some("ready" | "online" | "association" | "configuration")
                                )
                            }
                    })
                    .map(|(path, _)| path)
                    .ok_or("no matching ConnMan service")?;
                let operation = proxy(&conn, CM, path.as_str(), "net.connman.Service")?
                    .call::<_, _, ()>(
                        if action == "forget" {
                            "Remove"
                        } else {
                            "Disconnect"
                        },
                        &(),
                    )
                    .map_err(|e| e.to_string());
                operation
            }
        }
        _ => Err("not a native standalone backend".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_security_labels_do_not_downgrade_enterprise_or_wep() {
        assert_eq!(security("8021x"), "Enterprise");
        assert_eq!(security("wep"), "WEP");
        assert_eq!(security("sae"), "WPA3");
    }
    #[test]
    fn agents_refuse_other_networks_and_missing_authentication_fields() {
        let network = OwnedObjectPath::try_from("/network/one").unwrap();
        let other = OwnedObjectPath::try_from("/network/two").unwrap();
        let iwd = IwdAgent {
            network: network.clone(),
            password: "synthetic-only".into(),
        };
        assert!(iwd.request_passphrase(other.clone()).is_err());
        let cm = ConnManAgent {
            service: network,
            password: "synthetic-only".into(),
        };
        assert!(cm.request_input(other, HashMap::new()).is_err());
    }
}
