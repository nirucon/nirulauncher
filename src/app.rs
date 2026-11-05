//! The launcher's shared types and how a row starts a program.

#[allow(unused_imports)]
use crate::*;

#[derive(Clone, Copy)]
pub(crate) struct Rect {
    pub(crate) x: i16,
    pub(crate) y: i16,
    pub(crate) w: u16,
    pub(crate) h: u16,
}

#[derive(Clone)]
pub(crate) enum Action {
    Logout,
    Reboot,
    Shutdown,
    UpdateShutdown,
    UpdateReboot,
    ResetBar,
    ForgetWifi(String),
}

#[derive(Clone)]
pub(crate) enum Target {
    Spawn {
        argv: Vec<String>,
        terminal: bool,
        path: Option<std::path::PathBuf>,
    },
    /// Run in the configured terminal. `hold` keeps the window open after the
    /// child exits so reports and diagnostics can actually be read.
    Terminal {
        argv: Vec<String>,
        hold: bool,
    },
    /// Open a short prompt in a centred OpenCode terminal.
    Agent(String),
    Focus(Window),
    Confirm(Action),
    /// Switch the open launcher to another view without starting a second one.
    Mode(String),
    /// Run an explicit microphone test and show the result in the sound panel.
    SoundTest(u64),
    /// Copy this text to the clipboard through `xclip`.
    Copy(String),
    Info,
}

#[derive(Clone)]
pub(crate) struct Item {
    /// Stable key for favourites and local usage.
    pub(crate) key: String,
    pub(crate) group: String,
    pub(crate) name: String,
    pub(crate) detail: String,
    /// Extra searchable words from a desktop entry (Keywords, comment).
    pub(crate) keywords: String,
    pub(crate) favorite: bool,
    pub(crate) target: Target,
}

impl Item {
    pub(crate) fn label(&self) -> String {
        if self.group.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.group, self.name)
        }
    }

    #[cfg(test)]
    pub(crate) fn target_kind(&self) -> &'static str {
        match self.target {
            Target::Spawn { .. } => "spawn",
            Target::Terminal { .. } => "terminal",
            Target::Agent(_) => "agent",
            Target::Focus(_) => "focus",
            Target::Confirm(_) => "confirm",
            Target::Mode(_) => "mode",
            Target::SoundTest(_) => "sound-test",
            Target::Copy(_) => "copy",
            Target::Info => "info",
        }
    }
}

/// An inline numeric edit for the idle/lock timing view.
#[derive(Clone)]
pub(crate) struct NumberEdit {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) buffer: String,
}

/// A Wi-Fi password or hidden-network prompt. Secrets stay in memory only.
#[derive(Clone)]
pub(crate) struct WifiSecret {
    pub(crate) ssid: String,
    pub(crate) security: String,
    pub(crate) buffer: String,
    pub(crate) hidden: bool,
    pub(crate) stage: u8,
}

/// A choice list inside the Sound view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SoundPick {
    /// Choose the notification sound for one app.
    Sound { app: String },
    /// Choose an app to give its own sound.
    App,
}

/// What the event-sound worker reports.
pub(crate) enum EventsReply {
    /// The state after reading it or after one change, and NIRUX's answer to
    /// that change.
    State(
        std::result::Result<eventsound::Status, String>,
        Option<std::result::Result<String, String>>,
    ),
    /// App names seen in notifications, for "Add app…".
    Suggestions(Vec<String>),
}

/// Only a non-empty spawn row can become a saved action.
pub(crate) fn savable_argv(target: &Target) -> std::result::Result<&[String], &'static str> {
    match target {
        Target::Spawn { argv, .. } if !argv.is_empty() => Ok(argv),
        Target::Spawn { .. } => Err("This row has no command to save"),
        Target::Terminal { .. } => Err("Terminal actions cannot be saved as background actions"),
        Target::Agent(_) => Err("Agent prompts cannot be saved as background actions"),
        Target::Confirm(_) => Err("Confirmation actions are built in and cannot be saved"),
        Target::Mode(_) => Err("View switches cannot be saved as actions"),
        Target::SoundTest(_) => Err("Microphone tests cannot be saved as actions"),
        Target::Copy(_) => Err("Calculator results cannot be saved as actions"),
        Target::Info => Err("Info rows cannot be saved as actions"),
        Target::Focus(_) => Err("Window switches cannot be saved as actions"),
    }
}

/// Hand text to `xclip`, which keeps serving the clipboard after the
/// launcher exits. argv only; the text goes through stdin.
pub(crate) fn copy_to_clipboard(text: &str) -> std::result::Result<(), String> {
    use std::io::Write;
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-in"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start xclip: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("xclip has no input")?;
    stdin
        .write_all(text.as_bytes())
        .map_err(|e| format!("cannot copy: {e}"))?;
    Ok(())
}

pub(crate) fn spawn_command(
    argv: &[String],
    path: Option<&std::path::Path>,
    terminal: bool,
    cfg: &Config,
) -> std::result::Result<(), String> {
    if argv.is_empty() {
        return Err("empty command".into());
    }
    let mut full = if terminal {
        let mut v = cfg.terminal.clone();
        v.extend(argv.iter().cloned());
        v
    } else {
        argv.to_vec()
    };
    if full.is_empty() {
        return Err("empty command".into());
    }
    let program = full.remove(0);
    let mut command = Command::new(&program);
    command.args(&full);
    command.stdin(Stdio::null());
    if let Some(path) = path {
        command.current_dir(path);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {program}: {e}"))
}

/// The terminal command for an agent prompt. The prompt is one argv element,
/// never a shell fragment. Kitty and Alacritty use `--class`, Foot uses
/// `--app-id`; NIRUWM recognises `niru-agent` and centres it as a floating TUI.
pub(crate) fn agent_command(prompt: &str, cfg: &Config) -> Vec<String> {
    let mut full = cfg.terminal.clone();
    if full.is_empty() {
        return full;
    }
    let terminal = std::path::Path::new(&full[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let class_flag = match terminal.as_str() {
        "kitty" | "alacritty" | "wezterm" => Some("--class"),
        "foot" => Some("--app-id"),
        "xterm" | "urxvt" | "rxvt" => Some("-class"),
        _ => None,
    };
    if let Some(flag) = class_flag {
        full.push(flag.into());
        full.push("niru-agent".into());
    }
    full.extend(["niructl".into(), "agent".into(), prompt.into()]);
    full
}

pub(crate) fn spawn_agent(prompt: &str, cfg: &Config) -> std::result::Result<(), String> {
    let mut full = agent_command(prompt, cfg);
    if full.is_empty() {
        return Err("terminal command is empty".into());
    }
    let program = full.remove(0);
    Command::new(&program)
        .args(&full)
        .stdin(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {program}: {e}"))
}

/// Launch a terminal command. Informational actions keep the terminal open
/// after the child exits, with its exit status, so a report is not lost when
/// kitty/alacritty/foot closes the window immediately.
pub(crate) fn spawn_terminal_command(
    argv: &[String],
    hold: bool,
    cfg: &Config,
) -> std::result::Result<(), String> {
    if argv.is_empty() {
        return Err("empty command".into());
    }
    let mut full = cfg.terminal.clone();
    if hold {
        const HOLD: &str = "status=0; \"$@\" || status=$?; printf '\\n[exit %s] Press Enter to close...\\n' \"$status\"; read -r _ || :; exit \"$status\"";
        full.extend(["sh", "-c", HOLD, "sh"].into_iter().map(str::to_string));
    }
    full.extend(argv.iter().cloned());
    let program = full.remove(0);
    Command::new(&program)
        .args(&full)
        .stdin(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {program}: {e}"))
}
