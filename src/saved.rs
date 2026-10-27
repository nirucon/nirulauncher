//! Saved actions from `niructl action`.
//!
//! The launcher only ever stores an argv vector and a display name. It does not
//! write the action database itself; `niructl action save` owns that file.

use serde_json::Value;

/// Refuse to parse more than this much JSON, even if a helper misbehaves.
pub const MAX_JSON_BYTES: usize = 256 * 1024;

/// Upper bound for rows loaded from one helper invocation.
pub const MAX_ACTIONS: usize = 200;

const MAX_NAME_CHARS: usize = 80;
const MAX_ARGV_CHARS: usize = 64;
const MAX_ARG_CHARS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedAction {
    pub name: String,
    pub command: Vec<String>,
    pub favorite: bool,
}

fn clean_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|ch| !ch.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_NAME_CHARS).collect())
}

fn clean_command(command: &[Value]) -> Option<Vec<String>> {
    if command.is_empty() || command.len() > MAX_ARGV_CHARS {
        return None;
    }
    let mut argv = Vec::with_capacity(command.len());
    for value in command {
        let argument = value.as_str()?;
        if argument.contains('\0') || argument.chars().count() > MAX_ARG_CHARS {
            return None;
        }
        argv.push(argument.to_string());
    }
    let program = argv.first()?;
    if program.is_empty() {
        return None;
    }
    Some(argv)
}

/// Parse the documented `[{name, command, favorite}]` schema.
///
/// A malformed document or malformed row is ignored rather than guessed at.
/// Invalid rows inside an otherwise valid list are skipped individually.
pub fn parse(bytes: &[u8]) -> Vec<SavedAction> {
    if bytes.len() > MAX_JSON_BYTES {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let mut actions: Vec<SavedAction> = Vec::new();
    for row in rows.iter().take(MAX_ACTIONS) {
        let Some(name) = row.get("name").and_then(Value::as_str).and_then(clean_name) else {
            continue;
        };
        let Some(command) = row
            .get("command")
            .and_then(Value::as_array)
            .and_then(|argv| clean_command(argv))
        else {
            continue;
        };
        if actions
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        actions.push(SavedAction {
            name,
            command,
            favorite: row
                .get("favorite")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    actions
}

fn with_suffix(base: &str, suffix: usize) -> String {
    let suffix = format!(" {suffix}");
    let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    let stem: String = base.chars().take(keep).collect();
    format!("{}{suffix}", stem.trim_end())
}

/// Return `base`, or the first free `base N` name, without touching an
/// existing action. Comparison is case-insensitive because action names are
/// user-facing identifiers.
pub fn unique_name<'a, I>(base: &str, existing: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let base = clean_name(base).unwrap_or_else(|| "Saved action".to_string());
    let taken: std::collections::HashSet<String> = existing
        .into_iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    if !taken.contains(&base.to_lowercase()) {
        return base;
    }
    for suffix in 2..=9999 {
        let candidate = with_suffix(&base, suffix);
        if !taken.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    with_suffix(&base, 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_json_schema_is_parsed() {
        let actions = parse(
            br#"[
                {"name":"Night desk","command":["niructl","nightlight","on"],"favorite":true},
                {"name":"Editor","command":["/usr/bin/editor","file.txt"],"favorite":false}
            ]"#,
        );
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].name, "Night desk");
        assert_eq!(actions[0].command, ["niructl", "nightlight", "on"]);
        assert!(actions[0].favorite);
        assert_eq!(actions[1].command, ["/usr/bin/editor", "file.txt"]);
        assert!(!actions[1].favorite);
    }

    #[test]
    fn invalid_documents_and_rows_fail_soft() {
        assert!(parse(b"not json").is_empty());
        assert!(parse(b"{}").is_empty());
        assert!(parse(b"[]").is_empty());
        assert!(parse(&vec![b'x'; MAX_JSON_BYTES + 1]).is_empty());
        let actions = parse(
            br#"[
                {"name":"missing command"},
                {"name":"empty command","command":[]},
                {"name":"bad argv","command":"echo hi"},
                {"name":"bad arg","command":["echo", 2]},
                {"name":"null byte","command":["echo","a\u0000b"]},
                {"name":"  ","command":["echo"]},
                {"name":"good","command":["echo","ok"]}
            ]"#,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "good");
    }

    #[test]
    fn duplicate_names_are_loaded_once() {
        let actions = parse(
            br#"[
                {"name":"Same","command":["one"]},
                {"name":"same","command":["two"]}
            ]"#,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].command, ["one"]);
    }

    #[test]
    fn a_name_collision_gets_a_short_numeric_suffix() {
        let none: [&str; 0] = [];
        assert_eq!(unique_name("Editor", none), "Editor");
        let existing = ["Editor".to_string()];
        assert_eq!(
            unique_name("Editor", existing.iter().map(String::as_str)),
            "Editor 2"
        );
        let existing = ["editor".to_string(), "Editor 2".to_string()];
        assert_eq!(
            unique_name("Editor", existing.iter().map(String::as_str)),
            "Editor 3"
        );
        let existing = ["Editor".to_string()];
        assert_eq!(
            unique_name("Editor\nwith control", existing.iter().map(String::as_str)),
            "Editorwith control"
        );
    }

    #[test]
    fn empty_or_control_only_names_still_get_a_safe_fallback() {
        let none: [&str; 0] = [];
        assert_eq!(unique_name("", none), "Saved action");
        assert_eq!(unique_name("\n\t", none.iter().copied()), "Saved action");
        let existing = ["Saved action".to_string()];
        assert_eq!(
            unique_name("", existing.iter().map(String::as_str)),
            "Saved action 2"
        );
    }
}
