//! Discover XDG desktop entries and turn their Exec lines into argv arrays.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub argv: Vec<String>,
    pub path: Option<PathBuf>,
    pub terminal: bool,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    let system = std::env::var_os("XDG_DATA_DIRS")
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in system.split(':').filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir).join("applications"));
    }
    dirs
}

fn section_value(text: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            // Localized keys are not preferred here; English Name/Exec are
            // the stable contract, and a missing Name falls back to the ID.
            if !key.contains('[') {
                values.insert(key.to_string(), value.to_string());
            }
        }
    }
    values
}

fn bool_value(v: Option<&String>) -> bool {
    v.map(|s| s.eq_ignore_ascii_case("true")).unwrap_or(false)
}

fn in_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    path.to_string_lossy().split(':').any(|dir| {
        let p = Path::new(dir).join(program);
        p.is_file()
    })
}

fn desktop_allows(values: &HashMap<String, String>) -> bool {
    if bool_value(values.get("Hidden")) || bool_value(values.get("NoDisplay")) {
        return false;
    }
    if let Some(exec) = values.get("TryExec") {
        if !in_path(exec.trim()) {
            return false;
        }
    }
    if values
        .get("Type")
        .map(String::as_str)
        .unwrap_or("Application")
        != "Application"
    {
        return false;
    }
    let current = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "NIRUWM".into());
    let current: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    if let Some(only) = values.get("OnlyShowIn") {
        if !only
            .split(';')
            .filter(|s| !s.is_empty())
            .any(|s| current.contains(&s))
        {
            return false;
        }
    }
    if let Some(not) = values.get("NotShowIn") {
        if not
            .split(';')
            .filter(|s| !s.is_empty())
            .any(|s| current.contains(&s))
        {
            return false;
        }
    }
    true
}

/// Split a desktop-entry Exec value into arguments.
///
/// Double quotes group spaces; inside them `\"`, `\\`, `` ` ``, `$` and `\``
/// are unescaped as the specification requires. Outside quotes a backslash
/// also escapes the next character. Field codes are handled by `field_codes`.
pub fn parse_exec(exec: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut arg = String::new();
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                let next = chars.next()?;
                match next {
                    '"' | '\\' | '`' | '$' => arg.push(next),
                    _ => {
                        arg.push('\\');
                        arg.push(next);
                    }
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    arg.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !arg.is_empty() {
                    args.push(std::mem::take(&mut arg));
                }
            }
            c => arg.push(c),
        }
    }
    if quoted {
        return None;
    }
    if !arg.is_empty() {
        args.push(arg);
    }
    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}

fn field_codes(argv: Vec<String>, name: &str, icon: &str, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    for arg in argv {
        if arg.contains("%i") {
            if !icon.is_empty() {
                out.push("--icon".into());
                out.push(icon.into());
            }
            continue;
        }
        let mut replaced = String::new();
        let mut chars = arg.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                replaced.push(ch);
                continue;
            }
            match chars.next() {
                Some('%') => replaced.push('%'),
                Some('c') => replaced.push_str(name),
                Some('k') => replaced.push_str(path),
                Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
                Some(other) => {
                    replaced.push('%');
                    replaced.push(other);
                }
                None => replaced.push('%'),
            }
        }
        if !replaced.is_empty() {
            out.push(replaced);
        }
    }
    out
}

pub fn app_from_text(id: &str, text: &str) -> Option<App> {
    let values = section_value(text);
    if !desktop_allows(&values) {
        return None;
    }
    let name = values
        .get("Name")
        .map(String::as_str)
        .unwrap_or(id)
        .to_string();
    let exec = values.get("Exec")?;
    let raw = parse_exec(exec)?;
    let icon = values.get("Icon").map(String::as_str).unwrap_or("");
    let path_value = values.get("Path").map(String::as_str).unwrap_or("");
    let path = (!path_value.is_empty() && Path::new(path_value).is_dir())
        .then(|| PathBuf::from(path_value));
    let argv = field_codes(raw, &name, icon, path_value);
    Some(App {
        id: id.to_string(),
        name,
        argv,
        path,
        terminal: bool_value(values.get("Terminal")),
    })
}

/// Scan all application directories, keeping the first ID in XDG precedence.
pub fn discover() -> Vec<App> {
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for dir in data_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("desktop"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(id) = path.file_name().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.len() > 512 * 1024 {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(app) = app_from_text(&id, &text) {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

#[allow(dead_code)]
pub fn matching(apps: &[App], query: &str) -> Vec<App> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return apps.to_vec();
    }
    let mut hits: Vec<(u8, App)> = apps
        .iter()
        .filter_map(|app| {
            let name = app.name.to_lowercase();
            let id = app.id.to_lowercase();
            let rank = if name == query || id == query {
                0
            } else if name.starts_with(&query) {
                1
            } else if name.contains(&query) {
                2
            } else if id.contains(&query) {
                3
            } else {
                return None;
            };
            Some((rank, app.clone()))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    hits.into_iter().map(|(_, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quoting_and_field_codes() {
        let argv = parse_exec(r#"/usr/bin/app --name "two words" --literal 100%% %U"#).unwrap();
        assert_eq!(
            argv,
            vec![
                "/usr/bin/app",
                "--name",
                "two words",
                "--literal",
                "100%%",
                "%U"
            ]
        );
        let app = app_from_text(
            "x.desktop",
            "[Desktop Entry]\nType=Application\nName=Fish & Chips\nExec=/bin/app %c %k %i %f\nIcon=fish\nPath=/\n",
        )
        .unwrap();
        assert_eq!(app.name, "Fish & Chips");
        assert_eq!(
            app.argv,
            vec!["/bin/app", "Fish & Chips", "/", "--icon", "fish"]
        );
        assert_eq!(app.path.as_deref(), Some(Path::new("/")));
    }

    #[test]
    fn visibility_and_desktop_filters() {
        let base = "[Desktop Entry]\nType=Application\nExec=/bin/true\nName=Test\n";
        assert!(app_from_text("a.desktop", base).is_some());
        assert!(app_from_text("a.desktop", &format!("{base}NoDisplay=true\n")).is_none());
        assert!(app_from_text("a.desktop", &format!("{base}Hidden=true\n")).is_none());
        assert!(app_from_text("a.desktop", &format!("{base}OnlyShowIn=GNOME;\n")).is_none());
        assert!(app_from_text("a.desktop", &format!("{base}NotShowIn=NIRUWM;\n")).is_none());
        assert!(app_from_text(
            "a.desktop",
            &format!("{base}TryExec=/definitely/not/here\n")
        )
        .is_none());
    }

    #[test]
    fn search_prefers_exact_and_prefix() {
        let mk = |name: &str| App {
            id: format!("{name}.desktop"),
            name: name.into(),
            argv: vec!["true".into()],
            path: None,
            terminal: false,
        };
        let apps = vec![mk("Terminal"), mk("Text Editor"), mk("Files")];
        let hits = matching(&apps, "te");
        assert_eq!(hits[0].name, "Terminal");
        assert_eq!(hits[1].name, "Text Editor");
        let hits = matching(&apps, "files");
        assert_eq!(hits[0].name, "Files");
    }
}
