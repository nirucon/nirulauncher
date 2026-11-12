//! Quick note (`--note`, Super+Shift+N in NIRUWM).
//!
//! One line of text. Enter always adds it to the inbox file (`[note] file`,
//! `~/Notes/inbox.md` by default), which NIRUNOTE opens like any Markdown
//! file. The selected row decides where it goes as well: nowhere else (the
//! first row), a Discord webhook or a trusted device, both through NIRUSHARE.

use crate::*;
use std::io::Write as _;

pub(crate) const NOTE_LOCAL: &str = "note:local";
const NOTE_DISCORD: &str = "note:discord:";
const NOTE_DEVICE: &str = "note:device:";

/// The row that is there at once; NIRUSHARE's rows follow from the worker.
pub(crate) fn note_local_items(cfg: &Config) -> Vec<Item> {
    vec![Item {
        key: NOTE_LOCAL.into(),
        group: "Quick note".into(),
        name: "Save to the inbox".into(),
        detail: cfg.note_file.clone(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }]
}

/// Discord webhooks and trusted devices that a note can also go to.
pub(crate) fn note_share_items() -> Vec<Item> {
    if !in_path("nirushare") {
        return Vec::new();
    }
    let mut items = Vec::new();
    for name in modes::actions::discord_webhook_names() {
        items.push(Item {
            key: format!("{NOTE_DISCORD}{name}"),
            group: "Quick note".into(),
            name: format!("Save and post to Discord · {name}"),
            detail: "Through NIRUSHARE".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    let devices = providers::output("nirushare", &["devices"])
        .ok()
        .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    for device in devices {
        let Some(host) = device.get("host").and_then(|v| v.as_str()) else {
            continue;
        };
        let online = device
            .get("online")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        items.push(Item {
            key: format!("{NOTE_DEVICE}{host}"),
            group: "Quick note".into(),
            name: format!("Save and send to {host}"),
            detail: if online {
                "Online"
            } else {
                "Offline · queued delivery"
            }
            .into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    items
}

/// `~/` expanded against HOME.
pub(crate) fn note_path(file: &str) -> std::path::PathBuf {
    match file.strip_prefix("~/") {
        Some(rest) => {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
        }
        None => std::path::PathBuf::from(file),
    }
}

/// One Markdown list line with the local time: `- 2026-11-12 09:41 text`.
pub(crate) fn note_line(text: &str, when: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {when} {text}\n")
}

/// Add the note to the inbox; a new file starts with a heading.
pub(crate) fn save_note(cfg: &Config, text: &str) -> Result<std::path::PathBuf> {
    let path = note_path(&cfg.note_file);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let new = !path.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if new {
        file.write_all(b"# Inbox\n\n")?;
    }
    let when = local_minute();
    file.write_all(note_line(text, &when).as_bytes())?;
    Ok(path)
}

/// The local time as `YYYY-MM-DD HH:MM`, through libc so no date crate is needed.
fn local_minute() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r only writes into the tm we own.
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// The NIRUSHARE command for the selected row, or None for the inbox only.
pub(crate) fn note_share_argv(key: &str) -> Option<Vec<String>> {
    if let Some(name) = key.strip_prefix(NOTE_DISCORD) {
        return Some(vec![
            "nirushare".into(),
            "send".into(),
            "--to".into(),
            format!("discord:{name}"),
            "-".into(),
            "--yes".into(),
        ]);
    }
    key.strip_prefix(NOTE_DEVICE).map(|host| {
        vec![
            "nirushare".into(),
            "send".into(),
            "--stdin".into(),
            "--to".into(),
            host.to_string(),
        ]
    })
}

/// Hand the text to NIRUSHARE on standard input, without waiting for delivery.
pub(crate) fn share_note(argv: &[String], text: &str) -> Result<()> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    // Reap it in the background so it never lingers as a zombie.
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_one_list_line() {
        assert_eq!(
            note_line("  köp\nmjölk  ", "2026-11-12 09:41"),
            "- 2026-11-12 09:41 köp mjölk\n"
        );
    }

    #[test]
    fn rows_choose_the_nirushare_target() {
        assert_eq!(note_share_argv(NOTE_LOCAL), None);
        assert_eq!(
            note_share_argv("note:discord:friends").unwrap()[3],
            "discord:friends"
        );
        assert_eq!(note_share_argv("note:device:othala").unwrap()[4], "othala");
    }

    #[test]
    fn the_inbox_path_expands_home() {
        let _guard = crate::test_env_lock();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/test");
        assert_eq!(
            note_path("~/Notes/inbox.md"),
            std::path::PathBuf::from("/home/test/Notes/inbox.md")
        );
        assert_eq!(
            note_path("/tmp/x.md"),
            std::path::PathBuf::from("/tmp/x.md")
        );
        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
