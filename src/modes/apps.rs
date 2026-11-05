//! Applications and open windows.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn app_items(apps: Vec<desktop::App>) -> Vec<Item> {
    apps.into_iter()
        .map(|app| Item {
            key: app.id.clone(),
            group: "Applications".into(),
            name: app.name,
            detail: app.id,
            keywords: app.keywords.join(" "),
            favorite: false,
            target: Target::Spawn {
                argv: app.argv,
                terminal: app.terminal,
                path: app.path,
            },
        })
        .collect()
}

/// Open top-level windows from EWMH, for the action launcher's Windows group.
pub(crate) fn window_items(c: &RustConnection, root: Window) -> Vec<Item> {
    let Ok(atom) = intern(c, "_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    let Ok(reply) = c.get_property(false, root, atom, AtomEnum::ANY, 0, 128) else {
        return Vec::new();
    };
    let Ok(reply) = reply.reply() else {
        return Vec::new();
    };
    let windows: Vec<Window> = reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default();
    let name_atom = intern(c, "_NET_WM_NAME").ok();
    let utf8 = intern(c, "UTF8_STRING").ok();
    let desktop_atom = intern(c, "_NET_WM_DESKTOP").ok();
    let mut items = Vec::new();
    for window in windows {
        let class = c
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()
            .and_then(|r| r.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .unwrap_or_default();
        if class.contains("NIRULANCHER") || class.contains("NIRULOCK") {
            continue;
        }
        let title = name_atom
            .zip(utf8)
            .and_then(|(atom, utf8)| {
                c.get_property(false, window, atom, utf8, 0, 1024)
                    .ok()?
                    .reply()
                    .ok()
            })
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                c.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
                    .ok()?
                    .reply()
                    .ok()
                    .map(|r| String::from_utf8_lossy(&r.value).to_string())
            })
            .unwrap_or_else(|| format!("Window {window}"));
        let desktop = desktop_atom.and_then(|atom| {
            c.get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()?
                .value32()?
                .next()
        });
        let group = desktop
            .map(|desktop| format!("Workspace {}", desktop + 1))
            .unwrap_or_else(|| "Windows".into());
        items.push(Item {
            key: format!("window:{window}"),
            group,
            name: title,
            detail: "Switch to this window".into(),
            keywords: class.replace('\0', " "),
            favorite: false,
            target: Target::Focus(window),
        });
    }
    items.sort_by_key(|item| (item.group.clone(), item.name.to_lowercase()));
    items.truncate(64);
    items
}
