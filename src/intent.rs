//! Deterministic phrase routing for the launcher.
//!
//! This module only maps a bounded, normalized phrase to a closed set of
//! actions. It never builds a command from the query: callers resolve the
//! action against their own fixed argv catalogue. Unknown text, oversized
//! input, punctuation tricks and shell syntax therefore have no effect.

/// Longest accepted query, in characters. Ordinary search still gets the full
/// text; only intent resolution ignores overly long input.
pub const MAX_QUERY_CHARS: usize = 160;

/// At most this many intents are offered for one query.
pub const MAX_RESULTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntentAction {
    Lock,
    Logout,
    Reboot,
    Shutdown,
    FocusMode,
    PresentationMode,
    StopContext,
    Dnd,
    KeepAwake,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    NightLight,
    Compositor,
    WindowsRescue,
    WifiOn,
    WifiOff,
}

impl IntentAction {
    /// Stable identifier used for the row key and by tests.
    pub fn id(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Logout => "logout",
            Self::Reboot => "reboot",
            Self::Shutdown => "shutdown",
            Self::FocusMode => "focus-mode",
            Self::PresentationMode => "presentation-mode",
            Self::StopContext => "stop-context",
            Self::Dnd => "dnd",
            Self::KeepAwake => "keep-awake",
            Self::VolumeUp => "volume-up",
            Self::VolumeDown => "volume-down",
            Self::VolumeMute => "volume-mute",
            Self::MicMute => "mic-mute",
            Self::BrightnessUp => "brightness-up",
            Self::BrightnessDown => "brightness-down",
            Self::NightLight => "night-light",
            Self::Compositor => "compositor",
            Self::WindowsRescue => "windows-rescue",
            Self::WifiOn => "wifi-on",
            Self::WifiOff => "wifi-off",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IntentMatch {
    pub action: IntentAction,
    /// The catalogue phrase that matched exactly.
    pub phrase: &'static str,
}

struct Entry {
    action: IntentAction,
    phrases: &'static [&'static str],
}

/// English and Swedish variants, including common transliterations. Phrases
/// are intentionally specific; bare words such as "on", "off" or "stop" stay
/// ordinary searches instead of becoming commands.
const CATALOGUE: &[Entry] = &[
    Entry {
        action: IntentAction::Lock,
        phrases: &[
            "lock",
            "lock screen",
            "lås",
            "lås skärmen",
            "las",
            "las skarmen",
        ],
    },
    Entry {
        action: IntentAction::Logout,
        phrases: &["logout", "log out", "logga ut"],
    },
    Entry {
        action: IntentAction::Reboot,
        phrases: &[
            "reboot",
            "restart computer",
            "starta om",
            "starta om datorn",
        ],
    },
    Entry {
        action: IntentAction::Shutdown,
        phrases: &[
            "shutdown",
            "shut down",
            "shut down computer",
            "stäng av",
            "stäng av datorn",
            "stang av",
            "stang av datorn",
        ],
    },
    Entry {
        action: IntentAction::FocusMode,
        phrases: &["focus mode", "focus timer", "fokusläge", "fokuslage"],
    },
    Entry {
        action: IntentAction::PresentationMode,
        phrases: &[
            "presentation",
            "presentation mode",
            "mötesläge",
            "moteslage",
        ],
    },
    Entry {
        action: IntentAction::StopContext,
        phrases: &[
            "stop context",
            "stop work context",
            "avsluta läge",
            "avsluta lage",
            "avsluta kontext",
        ],
    },
    Entry {
        action: IntentAction::Dnd,
        phrases: &["dnd", "do not disturb", "stör ej", "stor ej"],
    },
    Entry {
        action: IntentAction::KeepAwake,
        phrases: &[
            "keep awake",
            "keep-awake",
            "håll vaken",
            "hall vaken",
            "håll datorn vaken",
            "hall datorn vaken",
        ],
    },
    Entry {
        action: IntentAction::VolumeUp,
        phrases: &[
            "volume up",
            "raise volume",
            "höj volymen",
            "hoj volymen",
            "volym upp",
        ],
    },
    Entry {
        action: IntentAction::VolumeDown,
        phrases: &[
            "volume down",
            "lower volume",
            "sänk volymen",
            "sank volymen",
            "volym ner",
        ],
    },
    Entry {
        action: IntentAction::VolumeMute,
        phrases: &[
            "volume mute",
            "mute",
            "mute audio",
            "mute volume",
            "ljud av",
            "tyst",
        ],
    },
    Entry {
        action: IntentAction::MicMute,
        phrases: &[
            "mic mute",
            "mute microphone",
            "microphone mute",
            "mikrofon av",
            "mikrofonen av",
            "mikrofon mute",
        ],
    },
    Entry {
        action: IntentAction::BrightnessUp,
        phrases: &[
            "brightness up",
            "brighter",
            "ljusare",
            "höj ljusstyrkan",
            "hoj ljusstyrkan",
        ],
    },
    Entry {
        action: IntentAction::BrightnessDown,
        phrases: &[
            "brightness down",
            "darker",
            "mörkare",
            "morkare",
            "sänk ljusstyrkan",
            "sank ljusstyrkan",
        ],
    },
    Entry {
        action: IntentAction::NightLight,
        phrases: &["night light", "nightlight", "nattljus"],
    },
    Entry {
        action: IntentAction::Compositor,
        phrases: &[
            "compositor",
            "toggle compositor",
            "compositor toggle",
            "kompositor",
        ],
    },
    Entry {
        action: IntentAction::WindowsRescue,
        phrases: &[
            "bring off-screen windows back",
            "bring off screen windows back",
            "rescue windows",
            "off-screen windows",
            "offscreen windows",
            "hämta tillbaka fönster",
            "hamta tillbaka fonster",
            "rädda fönster",
            "radda fonster",
            "återställ fönster",
            "aterstall fonster",
        ],
    },
    Entry {
        action: IntentAction::WifiOn,
        phrases: &[
            "wifi on",
            "wi-fi on",
            "turn on wifi",
            "turn wifi on",
            "enable wifi",
            "slå på wifi",
            "sla pa wifi",
            "wifi på",
            "wifi pa",
        ],
    },
    Entry {
        action: IntentAction::WifiOff,
        phrases: &[
            "wifi off",
            "wi-fi off",
            "turn off wifi",
            "turn wifi off",
            "disable wifi",
            "stäng av wifi",
            "stang av wifi",
            "wifi av",
        ],
    },
];

/// Lowercase, trim and collapse whitespace. Punctuation is treated as a word
/// boundary, so "wi-fi" and "off-screen" work without creating shell syntax.
/// Returns `None` for empty, oversized or otherwise unusable input.
pub fn normalize(query: &str) -> Option<String> {
    if query.chars().count() > MAX_QUERY_CHARS {
        return None;
    }
    let mut out = String::with_capacity(query.len());
    let mut pending_space = false;
    for ch in query.chars() {
        if ch.is_control() && !ch.is_whitespace() {
            return None;
        }
        // Shell and formatting metacharacters are rejected outright rather
        // than skimmed off as harmless punctuation.
        if matches!(
            ch,
            '$' | '`' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{' | '}' | '\\' | '"' | '\''
        ) {
            return None;
        }
        if ch.is_whitespace() || !ch.is_alphanumeric() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        for lowercase in ch.to_lowercase() {
            out.push(lowercase);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Resolve a query to at most [`MAX_RESULTS`] distinct actions.
///
/// Matching is exact after normalization. The catalogue order is fixed, so the
/// same query always produces the same rows, and duplicate aliases for one
/// action do not create duplicate rows.
pub fn resolve(query: &str) -> Vec<IntentMatch> {
    let Some(normalized) = normalize(query) else {
        return Vec::new();
    };
    let mut matches: Vec<IntentMatch> = Vec::new();
    for entry in CATALOGUE {
        let Some(phrase) = entry
            .phrases
            .iter()
            .find(|phrase| normalize(phrase).as_deref() == Some(normalized.as_str()))
        else {
            continue;
        };
        if matches.iter().any(|found| found.action == entry.action) {
            continue;
        }
        matches.push(IntentMatch {
            action: entry.action,
            phrase,
        });
        if matches.len() == MAX_RESULTS {
            break;
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_for(query: &str) -> Option<IntentAction> {
        resolve(query).first().map(|found| found.action)
    }

    #[test]
    fn english_phrases_map_to_their_action() {
        for (query, action) in [
            ("lock", IntentAction::Lock),
            ("lock screen", IntentAction::Lock),
            ("logout", IntentAction::Logout),
            ("reboot", IntentAction::Reboot),
            ("shutdown", IntentAction::Shutdown),
            ("focus mode", IntentAction::FocusMode),
            ("presentation mode", IntentAction::PresentationMode),
            ("stop context", IntentAction::StopContext),
            ("dnd", IntentAction::Dnd),
            ("keep awake", IntentAction::KeepAwake),
            ("volume up", IntentAction::VolumeUp),
            ("volume down", IntentAction::VolumeDown),
            ("mute", IntentAction::VolumeMute),
            ("mic mute", IntentAction::MicMute),
            ("brightness up", IntentAction::BrightnessUp),
            ("brightness down", IntentAction::BrightnessDown),
            ("night light", IntentAction::NightLight),
            ("compositor", IntentAction::Compositor),
            ("bring off-screen windows back", IntentAction::WindowsRescue),
            ("wifi on", IntentAction::WifiOn),
            ("wifi off", IntentAction::WifiOff),
        ] {
            assert_eq!(action_for(query), Some(action), "{query}");
        }
    }

    #[test]
    fn swedish_phrases_map_to_their_action() {
        for (query, action) in [
            ("lås", IntentAction::Lock),
            ("LÅS SKÄRMEN", IntentAction::Lock),
            ("logga ut", IntentAction::Logout),
            ("starta om", IntentAction::Reboot),
            ("stäng av", IntentAction::Shutdown),
            ("fokusläge", IntentAction::FocusMode),
            ("mötesläge", IntentAction::PresentationMode),
            ("avsluta läge", IntentAction::StopContext),
            ("stör ej", IntentAction::Dnd),
            ("håll vaken", IntentAction::KeepAwake),
            ("höj volymen", IntentAction::VolumeUp),
            ("sänk volymen", IntentAction::VolumeDown),
            ("ljud av", IntentAction::VolumeMute),
            ("mikrofon av", IntentAction::MicMute),
            ("ljusare", IntentAction::BrightnessUp),
            ("mörkare", IntentAction::BrightnessDown),
            ("nattljus", IntentAction::NightLight),
            ("hämta tillbaka fönster", IntentAction::WindowsRescue),
            ("slå på wifi", IntentAction::WifiOn),
            ("stäng av wifi", IntentAction::WifiOff),
        ] {
            assert_eq!(action_for(query), Some(action), "{query}");
        }
    }

    #[test]
    fn case_whitespace_and_punctuation_are_tolerated() {
        for query in [
            "  LOCK   SCREEN  ",
            "wi-fi ON",
            "bring off-screen windows back!",
            "\tlås\t",
        ] {
            assert!(!resolve(query).is_empty(), "{query:?}");
        }
        assert_eq!(action_for("  LOCK   SCREEN  "), Some(IntentAction::Lock));
        assert_eq!(action_for("wi-fi ON"), Some(IntentAction::WifiOn));
    }

    #[test]
    fn hostile_or_unknown_input_never_routes_or_builds_a_command() {
        for query in [
            "lock; rm -rf /",
            "$(reboot)",
            "`shutdown`",
            "lock && reboot",
            "lås\nreboot",
            "wifi on 192.0.2.1",
            "{}",
            "🔥",
            &"lock ".repeat(MAX_QUERY_CHARS),
        ] {
            assert!(resolve(query).is_empty(), "{query:?}");
        }
        assert!(normalize(&"x".repeat(MAX_QUERY_CHARS + 1)).is_none());
        assert!(normalize("lock").is_some());
    }

    #[test]
    fn generic_words_stay_ordinary_searches() {
        for query in ["stop", "on", "off", "up", "down", "av", "läge", "restart"] {
            assert!(resolve(query).is_empty(), "{query} must not route");
        }
    }

    #[test]
    fn results_are_bounded_unique_and_deterministic() {
        let first = resolve("lock");
        let second = resolve("lock");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].action, second[0].action);
        assert_eq!(first[0].phrase, second[0].phrase);

        for entry in CATALOGUE {
            for phrase in entry.phrases {
                let found = resolve(phrase);
                assert!(found.len() <= MAX_RESULTS, "{phrase}");
                let mut actions: Vec<_> = found.iter().map(|item| item.action).collect();
                let total = actions.len();
                actions.sort_by_key(|action| action.id());
                actions.dedup();
                assert_eq!(actions.len(), total, "{phrase} produced duplicate actions");
            }
        }
    }

    #[test]
    fn no_phrase_is_shared_between_actions() {
        let mut seen: Vec<(String, IntentAction)> = Vec::new();
        for entry in CATALOGUE {
            for phrase in entry.phrases {
                let normalized = normalize(phrase).expect("catalogue phrase is valid");
                if let Some((_, other)) = seen.iter().find(|(text, _)| text == &normalized) {
                    assert_eq!(*other, entry.action, "{phrase:?} is ambiguous");
                } else {
                    seen.push((normalized, entry.action));
                }
            }
        }
    }
}
