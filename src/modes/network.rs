//! Wi-Fi and wired network views.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn wifi_items(snapshot: &wifi::Snapshot) -> Vec<Item> {
    let mut items = Vec::new();
    let group = "Wi-Fi";
    if !snapshot.present {
        return vec![Item {
            key: "wifi:absent".into(),
            group: group.into(),
            name: "No Wi-Fi hardware".into(),
            detail: "This computer has no wireless adapter".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }];
    }
    if snapshot.hard_blocked {
        items.push(wifi_info(
            "Wi-Fi is blocked by hardware",
            "Use the physical wireless switch or airplane-mode key",
        ));
    } else if snapshot.soft_blocked || !snapshot.powered {
        items.push(Item {
            key: "wifi:power".into(),
            group: group.into(),
            name: "Turn Wi-Fi on".into(),
            detail: if snapshot.hard_blocked {
                "Hardware blocked"
            } else {
                "Currently off"
            }
            .into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    } else {
        items.push(Item {
            key: "wifi:power".into(),
            group: group.into(),
            name: "Turn Wi-Fi off".into(),
            detail: format!("{} · on", snapshot.interface),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }

    if snapshot.connected {
        let signal = snapshot
            .signal
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "signal unknown".into());
        let detail = if snapshot.address.is_empty() {
            format!("{signal} · {}", snapshot.security)
        } else {
            format!("{signal} · {} · {}", snapshot.security, snapshot.address)
        };
        items.push(Item {
            key: "wifi:connected".into(),
            group: group.into(),
            name: format!("Connected: {}", snapshot.ssid),
            detail,
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: "wifi:disconnect".into(),
            group: group.into(),
            name: format!("Disconnect from {}", snapshot.ssid),
            detail: "Disconnect this Wi-Fi link; keep the saved profile".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        if snapshot.saved.contains(&snapshot.ssid) && snapshot.active_profile.is_empty() {
            items.push(Item {
                key: format!("wifi:forget:{}", snapshot.ssid),
                group: group.into(),
                name: format!("Forget {}", snapshot.ssid),
                detail: "Remove the saved profile".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
    }

    for profile in &snapshot.profiles {
        items.push(Item {
            key: format!("wifi:profile:{}", profile.id),
            group: group.into(),
            name: format!("Saved: {}", profile.ssid),
            detail: format!(
                "{} · {}{}",
                profile.security,
                profile.id.rsplit('/').next().unwrap_or("profile"),
                if profile.id == snapshot.active_profile {
                    " · active"
                } else {
                    ""
                }
            ),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: format!("wifi:forget:{}", profile.id),
            group: group.into(),
            name: format!("Forget saved profile: {}", profile.ssid),
            detail: profile.id.clone(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }

    for network in &snapshot.networks {
        if network.active {
            continue;
        }
        let signal = network
            .signal
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "not in range".into());
        let mut detail = format!("{signal} · {}", network.security);
        if network.saved {
            detail.push_str(" · saved");
        }
        if network.security.contains("Enterprise") {
            detail.push_str(" · saved profile required");
        }
        items.push(Item {
            key: format!("wifi:connect:{}", network.ssid),
            group: group.into(),
            name: network.ssid.clone(),
            detail,
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        if network.saved && network.needs_secret() {
            items.push(Item {
                key: format!("wifi:connect-pw:{}", network.ssid),
                group: group.into(),
                name: format!("Use password for {}", network.ssid),
                detail: "Saved profile · enter a new password".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
        if network.saved && snapshot.manager != "NetworkManager" {
            items.push(Item {
                key: format!("wifi:forget:{}", network.ssid),
                group: group.into(),
                name: format!("Forget {}", network.ssid),
                detail: "Remove the saved profile".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
    }

    if snapshot.powered && !snapshot.hard_blocked && !snapshot.soft_blocked {
        items.push(Item {
            key: "wifi:hidden-wpa".into(),
            group: group.into(),
            name: "Hidden network…".into(),
            detail: "Enter an SSID and WPA2/WPA3 password".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: "wifi:hidden-open".into(),
            group: group.into(),
            name: "Hidden open network…".into(),
            detail: "Enter an SSID; no password".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    items.push(Item {
        key: "wifi:refresh".into(),
        group: group.into(),
        name: "Rescan networks".into(),
        detail: "Look for networks again".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    });
    if items.is_empty() {
        items.push(wifi_info(
            "No networks found",
            "Move closer to an access point or rescan",
        ));
    }
    items
}

/// Search SSIDs, not command labels, security text or launcher favourites.
pub(crate) fn wifi_filtered(items: &[Item], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (query.is_empty()
                || ((item.key.starts_with("wifi:connect:")
                    || item.key.starts_with("wifi:profile:")
                    || item.key == "wifi:connected")
                    && item.name.to_lowercase().contains(&query)))
            .then_some(index)
        })
        .collect()
}

pub(crate) fn wired_items(snapshot: &wifi::WiredSnapshot) -> Vec<Item> {
    let mut items = Vec::new();
    for device in &snapshot.devices {
        let address = if device.address.is_empty() {
            "IP pending".to_string()
        } else {
            device.address.clone()
        };
        let link = if device.connected {
            "connected".to_string()
        } else if !device.carrier {
            "cable unplugged".to_string()
        } else {
            device.state.clone()
        };
        let mut detail = format!("{link} · {address}");
        if !device.connection.is_empty() {
            detail.push_str(&format!(" · {}", device.connection));
        }
        if let Some(speed) = device.speed.filter(|speed| *speed > 0) {
            detail.push_str(&format!(" · {speed} Mb/s"));
        }
        if !device.managed && snapshot.manager == "NetworkManager" {
            detail.push_str(" · unmanaged");
        }
        items.push(Item {
            key: format!("wired:info:{}", device.interface),
            group: "Wired".into(),
            name: format!("Wired: {}", device.interface),
            detail,
            keywords: "ethernet wired dock cable network".into(),
            favorite: false,
            target: Target::Info,
        });
        if snapshot.manager == "NetworkManager" && device.managed {
            if device.connected {
                items.push(Item {
                    key: format!("wired:disconnect:{}", device.interface),
                    group: "Wired".into(),
                    name: format!("Disconnect wired {}", device.interface),
                    detail: "Only this wired connection is changed; Wi-Fi stays untouched".into(),
                    keywords: "ethernet wired disconnect".into(),
                    favorite: false,
                    target: Target::Info,
                });
            } else {
                items.push(Item {
                    key: format!("wired:connect:{}", device.interface),
                    group: "Wired".into(),
                    name: format!("Connect wired {}", device.interface),
                    detail: "Use the existing NetworkManager wired profile".into(),
                    keywords: "ethernet wired connect".into(),
                    favorite: false,
                    target: Target::Info,
                });
            }
        }
    }
    if snapshot.manager != "NetworkManager" && snapshot.available {
        let detail = if snapshot.manager.starts_with("dhcpcd") {
            "dhcpcd configures a connected cable automatically; NIRUX will not change its routes or DHCP"
        } else {
            "No NetworkManager here; NIRUX will not change routes, DHCP or another manager"
        };
        items.push(wifi_info(
            if snapshot.manager.starts_with("dhcpcd") {
                "Wired is automatic (dhcpcd)"
            } else {
                "Wired is read-only"
            },
            detail,
        ));
    }
    if items.is_empty() {
        items.push(wifi_info(
            "No wired Ethernet interface",
            "Connect a dock or cable and rescan",
        ));
    }
    items.push(Item {
        key: "wired:refresh".into(),
        group: "Wired".into(),
        name: "Refresh wired network".into(),
        detail: "Re-read the physical link and NetworkManager state".into(),
        keywords: "ethernet cable dock refresh".into(),
        favorite: false,
        target: Target::Info,
    });
    items
}

pub(crate) fn notify_network(title: &str, body: &str, icon: &str) {
    let title = title.to_string();
    let body = body.to_string();
    let icon = icon.to_string();
    thread::spawn(move || {
        let _ = Command::new("notify-send")
            .args([
                "-a",
                "NIRU Network",
                "-u",
                "normal",
                "-i",
                &icon,
                &title,
                &body,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

pub(crate) fn wifi_info(name: &str, detail: &str) -> Item {
    Item {
        key: format!("wifi:info:{name}"),
        group: "Wi-Fi".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }
}

impl Ui {
    /// Open the Wi-Fi view with a quick status, then scan in a worker.
    pub(crate) fn enter_wifi(&mut self) -> Result<()> {
        self.catalog_rx = None;
        self.mode = "wifi".into();
        self.query.clear();
        self.confirm = None;
        self.number_edit = None;
        self.profile_name = None;
        self.wifi_secret = None;
        self.error.clear();
        // Mapping the window must not wait for D-Bus or privilege prompts.
        self.wifi = Some(wifi::Snapshot {
            present: wifi::hardware_present(),
            powered: true,
            ..Default::default()
        });
        self.refresh_wifi_items();
        if let Some(index) = self
            .filtered
            .iter()
            .position(|&index| self.items[index].key == "wifi:connected")
        {
            self.selected = index;
        }
        self.start_wifi_job("Scanning for networks…".into(), || Ok(()))
    }

    pub(crate) fn refresh_wifi_items(&mut self) {
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        let Some(snapshot) = &self.wifi else {
            return;
        };
        self.items = wifi_items(snapshot);
        self.plain_items = self.items.len();
        self.filtered = wifi_filtered(&self.items, &self.query);
        self.selected = previous
            .and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&index| self.items[index].key == key)
            })
            .unwrap_or(0)
            .min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
    }

    pub(crate) fn start_wifi_job<F>(&mut self, label: String, job: F) -> Result<()>
    where
        F: FnOnce() -> std::result::Result<(), String> + Send + 'static,
    {
        if self.wifi_rx.is_some() {
            self.error = "Wait for the current Wi-Fi operation".into();
            self.draw()?;
            return Ok(());
        }
        if label.starts_with("Connecting") || label.starts_with("Activating saved profile") {
            self.wifi_connect_pending = true;
        }
        let (tx, rx) = mpsc::channel();
        self.wifi_rx = Some(rx);
        self.wifi_busy = Some(label);
        self.error.clear();
        thread::spawn(move || {
            let result = job().map(|_| wifi::snapshot(true));
            let _ = tx.send(result);
        });
        self.draw()
    }

    pub(crate) fn poll_wifi(&mut self) -> Result<bool> {
        let Some(rx) = self.wifi_rx.as_ref() else {
            return Ok(false);
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The Wi-Fi worker stopped unexpectedly".into())
            }
        };
        self.wifi_rx = None;
        self.wifi_busy = None;
        match result {
            Ok(snapshot) => {
                let first_scan = self.wifi.as_ref().is_some_and(|old| old.manager.is_empty());
                if self.wifi_connect_pending {
                    if snapshot.connected {
                        notify_network(
                            "Wi-Fi connected",
                            &format!(
                                "{}{}\n{} · {}\n{}",
                                snapshot.ssid,
                                if snapshot.security.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", snapshot.security)
                                },
                                snapshot.interface,
                                snapshot.manager,
                                if snapshot.address.is_empty() {
                                    "IP pending".to_string()
                                } else {
                                    snapshot.address.clone()
                                }
                            ),
                            "network-wireless",
                        );
                    }
                    self.wifi_connect_pending = false;
                }
                self.error = snapshot.error.clone();
                self.wifi = Some(snapshot);
                self.refresh_wifi_items();
                if first_scan && self.query.trim().is_empty() {
                    self.selected = self
                        .filtered
                        .iter()
                        .position(|&index| self.items[index].key == "wifi:connected")
                        .unwrap_or(0);
                }
            }
            Err(error) => self.error = error,
        }
        self.draw()?;
        Ok(true)
    }

    pub(crate) fn enter_wired(&mut self) -> Result<()> {
        self.catalog_rx = None;
        self.mode = "wired".into();
        self.query.clear();
        self.confirm = None;
        self.number_edit = None;
        self.profile_name = None;
        self.wifi_secret = None;
        self.error.clear();
        self.wired = Some(wifi::wired_snapshot());
        self.refresh_wired_items();
        self.start_wired_job("Refreshing wired network…".into(), || Ok(()))
    }

    pub(crate) fn refresh_wired_items(&mut self) {
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        let Some(snapshot) = &self.wired else {
            return;
        };
        self.items = wired_items(snapshot);
        self.plain_items = self.items.len();
        self.filtered = filtered_for_mode(&self.items, "", &self.usage, "wired");
        self.selected = previous
            .and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&index| self.items[index].key == key)
            })
            .unwrap_or(0);
        self.offset = 0;
    }

    pub(crate) fn start_wired_job<F>(&mut self, label: String, job: F) -> Result<()>
    where
        F: FnOnce() -> std::result::Result<(), String> + Send + 'static,
    {
        if self.wired_rx.is_some() {
            self.error = "Wait for the current wired operation".into();
            self.draw()?;
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        self.wired_rx = Some(rx);
        self.wired_busy = Some(label);
        self.error.clear();
        thread::spawn(move || {
            let result = job().map(|_| wifi::wired_snapshot());
            let _ = tx.send(result);
        });
        self.draw()
    }

    pub(crate) fn poll_wired(&mut self) -> Result<bool> {
        let Some(rx) = self.wired_rx.as_ref() else {
            return Ok(false);
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The wired worker stopped unexpectedly".into())
            }
        };
        self.wired_rx = None;
        self.wired_busy = None;
        match result {
            Ok(snapshot) => {
                if let Some(interface) = self.wired_connect_pending.take() {
                    if let Some(device) = snapshot
                        .devices
                        .iter()
                        .find(|device| device.interface == interface && device.connected)
                    {
                        notify_network(
                            "Wired connected",
                            &format!(
                                "{}{}
{} · {}",
                                if device.connection.is_empty() {
                                    interface.clone()
                                } else {
                                    device.connection.clone()
                                },
                                if device.address.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", device.address)
                                },
                                device.interface,
                                device.state
                            ),
                            "network-wired",
                        );
                    }
                }
                self.error = snapshot.error.clone();
                self.wired = Some(snapshot);
                self.refresh_wired_items();
            }
            Err(error) => {
                self.wired_connect_pending = None;
                self.error = error;
            }
        }
        self.draw()?;
        Ok(true)
    }

    pub(crate) fn wired_execute(&mut self, key: &str) -> Result<Option<bool>> {
        if let Some(interface) = key.strip_prefix("wired:connect:") {
            let interface = interface.to_string();
            self.wired_connect_pending = Some(interface.clone());
            self.start_wired_job(format!("Connecting wired {interface}…"), move || {
                wifi::wired_connect(&interface)
            })?;
            return Ok(Some(true));
        }
        if let Some(interface) = key.strip_prefix("wired:disconnect:") {
            let interface = interface.to_string();
            self.start_wired_job(format!("Disconnecting wired {interface}…"), move || {
                wifi::wired_disconnect(&interface)
            })?;
            return Ok(Some(true));
        }
        if key == "wired:refresh" {
            self.start_wired_job("Refreshing wired network…".into(), || Ok(()))?;
            return Ok(Some(true));
        }
        Ok(None)
    }

    /// Handle a Wi-Fi row. `None` means this was not a Wi-Fi action.
    pub(crate) fn wifi_execute(&mut self, key: &str) -> Result<Option<bool>> {
        if let Some(id) = key.strip_prefix("wifi:profile:") {
            let id = id.to_owned();
            self.start_wifi_job("Activating saved profile…".into(), move || {
                wifi::connect_profile(&id)
            })?;
            return Ok(Some(true));
        }
        match key {
            "wifi:refresh" => {
                self.start_wifi_job("Scanning for networks…".into(), || Ok(()))?;
                return Ok(Some(true));
            }
            "wifi:power" => {
                let on = self.wifi.as_ref().map(|s| !s.powered).unwrap_or(true);
                let label = if on {
                    "Turning Wi-Fi on…"
                } else {
                    "Turning Wi-Fi off…"
                };
                self.start_wifi_job(label.into(), move || wifi::set_power(on))?;
                return Ok(Some(true));
            }
            "wifi:disconnect" => {
                self.start_wifi_job("Disconnecting…".into(), wifi::disconnect)?;
                return Ok(Some(true));
            }
            "wifi:connected" => return Ok(Some(true)),
            "wifi:hidden-wpa" | "wifi:hidden-open" => {
                self.wifi_secret = Some(WifiSecret {
                    ssid: String::new(),
                    security: if key == "wifi:hidden-open" {
                        "Open".into()
                    } else {
                        "WPA2/WPA3".into()
                    },
                    buffer: String::new(),
                    hidden: true,
                    stage: 0,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
            _ => {}
        }
        if let Some(ssid) = key.strip_prefix("wifi:forget:") {
            self.confirm = Some(Action::ForgetWifi(ssid.to_string()));
            self.draw()?;
            return Ok(Some(true));
        }
        if let Some(ssid) = key.strip_prefix("wifi:connect-pw:") {
            if let Some(network) = self
                .wifi
                .as_ref()
                .and_then(|snapshot| snapshot.networks.iter().find(|n| n.ssid == ssid))
            {
                self.wifi_secret = Some(WifiSecret {
                    ssid: network.ssid.clone(),
                    security: network.security.clone(),
                    buffer: String::new(),
                    hidden: false,
                    stage: 1,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
        }
        if let Some(ssid) = key.strip_prefix("wifi:connect:") {
            let Some(network) = self
                .wifi
                .as_ref()
                .and_then(|snapshot| snapshot.networks.iter().find(|n| n.ssid == ssid))
                .cloned()
            else {
                return Ok(Some(true));
            };
            if !network.can_connect() {
                self.error = "this network needs a saved enterprise profile".into();
                self.draw()?;
                return Ok(Some(true));
            }
            if network.saved && !network.security.contains("Enterprise") {
                let ssid = network.ssid.clone();
                self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                    wifi::connect(&ssid, "Saved", "", false)
                })?;
                return Ok(Some(true));
            }
            if network.needs_secret() {
                self.wifi_secret = Some(WifiSecret {
                    ssid: network.ssid.clone(),
                    security: network.security.clone(),
                    buffer: String::new(),
                    hidden: false,
                    stage: 1,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
            let ssid = network.ssid.clone();
            let security = network.security.clone();
            self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                wifi::connect(&ssid, &security, "", false)
            })?;
            return Ok(Some(true));
        }
        Ok(None)
    }

    pub(crate) fn wifi_secret_key(&mut self, key: u32) -> Result<()> {
        let Some(mut secret) = self.wifi_secret.take() else {
            return Ok(());
        };
        match key {
            0xff1b => {
                self.error.clear();
            }
            0xff0d | 0xff8d => {
                if secret.hidden && secret.stage == 0 {
                    let ssid = secret.buffer.trim().to_string();
                    if ssid.is_empty() {
                        self.error = "Enter the hidden network name".into();
                        self.wifi_secret = Some(secret);
                    } else {
                        secret.ssid = ssid;
                        secret.stage = 1;
                        secret.buffer.clear();
                        if secret.security == "Open" {
                            let ssid = secret.ssid.clone();
                            self.wifi_secret = None;
                            self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                                wifi::connect(&ssid, "Open", "", true)
                            })?;
                            return Ok(());
                        }
                        self.wifi_secret = Some(secret);
                    }
                } else {
                    let ssid = secret.ssid.clone();
                    let security = secret.security.clone();
                    let password = secret.buffer.clone();
                    self.wifi_secret = None;
                    self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                        wifi::connect(&ssid, &security, &password, secret.hidden)
                    })?;
                    return Ok(());
                }
            }
            0xff08 => {
                secret.buffer.pop();
                self.wifi_secret = Some(secret);
            }
            _ if is_text_key(key) => {
                if let Some(ch) = key_char(key) {
                    if !ch.is_control() && (secret.hidden && secret.stage == 0 || ch != '\0') {
                        secret.buffer.push(ch);
                    }
                }
                self.wifi_secret = Some(secret);
            }
            _ => self.wifi_secret = Some(secret),
        }
        self.draw()
    }
}

#[cfg(test)]
mod search_tests {
    #![allow(dead_code, unused_imports)]
    use crate::{filtered, Item, Target};

    const DAY: u64 = 86_400;

    fn row(group: &str, name: &str, detail: &str) -> Item {
        Item {
            key: format!("{group}:{name}"),
            group: group.into(),
            name: name.into(),
            detail: detail.into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }
    }

    fn names(items: &[Item], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| items[i].name.clone()).collect()
    }

    /// The dedicated mode lists every provider before anything is typed, and
    /// no applications at all.
    #[test]
    fn wifi_view_offers_power_and_refresh_without_an_adapter() {
        let rows = super::wifi_items(&crate::wifi::Snapshot::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "wifi:absent");
    }

    #[test]
    fn wifi_search_matches_ssids_instead_of_action_labels_and_clears() {
        let snapshot = crate::wifi::Snapshot {
            present: true,
            powered: true,
            networks: vec![
                crate::wifi::Network {
                    ssid: "Räknät hemma".into(),
                    security: "WPA2".into(),
                    ..Default::default()
                },
                crate::wifi::Network {
                    ssid: "Other".into(),
                    security: "WPA2".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rows = super::wifi_items(&snapshot);
        let matches = super::wifi_filtered(&rows, "RÄKNÄT");
        assert_eq!(matches.len(), 1);
        assert_eq!(rows[matches[0]].name, "Räknät hemma");
        assert!(super::wifi_filtered(&rows, "WPA2").is_empty());
        assert_eq!(super::wifi_filtered(&rows, "").len(), rows.len());
    }

    #[test]
    fn wifi_view_marks_saved_networks_and_skips_the_active_one() {
        let snapshot = crate::wifi::Snapshot {
            present: true,
            interface: "wlan0".into(),
            powered: true,
            connected: true,
            ssid: "Home".into(),
            signal: Some(70),
            security: "WPA2".into(),
            saved: vec!["Home".into(), "Guest".into()],
            networks: vec![
                crate::wifi::Network {
                    ssid: "Home".into(),
                    active: true,
                    saved: true,
                    ..Default::default()
                },
                crate::wifi::Network {
                    ssid: "Guest".into(),
                    saved: true,
                    security: "WPA2".into(),
                    signal: Some(45),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rows = super::wifi_items(&snapshot);
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert!(keys.contains(&"wifi:power"));
        assert!(keys.contains(&"wifi:disconnect"));
        assert!(keys.contains(&"wifi:forget:Home"));
        assert!(keys.contains(&"wifi:connect:Guest"));
        assert!(keys.contains(&"wifi:connect-pw:Guest"));
        assert!(!keys.contains(&"wifi:connect:Home"));
        assert!(keys.contains(&"wifi:refresh"));
    }
}
