//! The NIRULOCK handoff contract (F01): when the lock asks the selection owner
//! of `_NIRULANCHER_SELECTION` to yield with a `_NIRULANCHER_LOCK` client
//! message, the launcher drops its grabs and the selection well within
//! NIRULOCK's 800 ms deadline, also while provider rows are still loading.
//!
//! Needs a private X server without a window manager:
//! `xvfb-run -a cargo test --test lock_handoff -- --ignored`

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

fn atom(c: &RustConnection, name: &str) -> Atom {
    c.intern_atom(false, name.as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom
}

fn owner(c: &RustConnection, selection: Atom) -> Window {
    c.get_selection_owner(selection)
        .unwrap()
        .reply()
        .unwrap()
        .owner
}

struct Fixture {
    dir: std::path::PathBuf,
    child: Child,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Start the launcher with providers that answer after `sleep` seconds.
fn launch(mode: &str, sleep: u32) -> Fixture {
    let dir = std::env::temp_dir().join(format!("nl-lock-{}-{mode}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    let apps = dir.join("data/applications");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("lock.desktop"),
        "[Desktop Entry]\nType=Application\nName=Lock fixture\nExec=true\n",
    )
    .unwrap();
    for name in ["niructl", "nirux", "niruclip", "nirunotify"] {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\nsleep {sleep}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(env!("CARGO_BIN_EXE_nirulauncher"))
        .arg(format!("--{mode}"))
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/.config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("NIRU_UI_SCALE", "1")
        .env("PATH", path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Fixture { dir, child }
}

fn handoff(mode: &str, sleep: u32) {
    let (c, screen) = x11rb::connect(None).unwrap();
    let root = c.setup().roots[screen].root;
    let selection = atom(&c, "_NIRULANCHER_SELECTION");
    let request = atom(&c, "_NIRULANCHER_LOCK");
    let mut fixture = launch(mode, sleep);
    let deadline = Instant::now() + Duration::from_secs(5);
    let launcher = loop {
        let window = owner(&c, selection);
        if window != x11rb::NONE {
            break window;
        }
        assert!(
            Instant::now() < deadline,
            "{mode}: the launcher never opened"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    // Let it take its grabs, then ask exactly as NIRULOCK 0.17.2+ does.
    std::thread::sleep(Duration::from_millis(150));
    let asked = Instant::now();
    c.send_event(
        false,
        launcher,
        EventMask::NO_EVENT,
        ClientMessageEvent::new(32, launcher, request, [1, 0, 0, 0, 0]),
    )
    .unwrap();
    c.flush().unwrap();
    while owner(&c, selection) != x11rb::NONE {
        assert!(
            asked.elapsed() < Duration::from_millis(800),
            "{mode}: the launcher kept the selection past NIRULOCK's deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let released = asked.elapsed();
    assert!(
        released < Duration::from_millis(300),
        "{mode}: yielding took {released:?}"
    );
    // The lock can now take the keyboard and the pointer.
    let keyboard = c
        .grab_keyboard(
            false,
            root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(keyboard.status, GrabStatus::SUCCESS, "{mode}: keyboard");
    let pointer = c
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(pointer.status, GrabStatus::SUCCESS, "{mode}: pointer");
    c.ungrab_keyboard(x11rb::CURRENT_TIME).unwrap();
    c.ungrab_pointer(x11rb::CURRENT_TIME).unwrap();
    c.flush().unwrap();
    let exit_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if fixture.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < exit_deadline,
            "{mode}: the process did not exit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires a private X server without another window manager"]
fn the_launcher_yields_to_the_lock_while_providers_load() {
    handoff("actions", 3);
    handoff("health", 3);
    handoff("sound", 3);
}

#[test]
#[ignore = "requires a private X server without another window manager"]
fn the_launcher_yields_to_the_lock_when_idle() {
    handoff("apps", 0);
    handoff("settings", 0);
}
