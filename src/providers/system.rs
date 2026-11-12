//! System providers shared by several views: programs on PATH, the NIRUX
//! generation and update cache, and saved actions.

use crate::*;

/// Provider queries in a view switch must not own the UI indefinitely.
///
/// GNU timeout bounds the child and its output wait; a provider that hangs is
/// shown as an unavailable row instead of freezing Escape and the lock handoff.
pub(crate) fn bounded_output(
    program: &str,
    args: &[&str],
    budget: std::time::Duration,
) -> Option<std::process::Output> {
    providers::bounded(program, args, budget)
}

/// The launcher changed something itself: later views read providers again.
pub(crate) fn forget_providers() {
    providers::forget();
    sound::forget_status();
}

/// Find an installed program the same way the shell would.
///
/// `~/.local/bin` is on the session path, but the launcher may be started
/// before that is set, so it is checked explicitly.
pub(crate) fn in_path(name: &str) -> bool {
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".local/share/nirux/current/bin"));
    }
    dirs.iter().any(|dir| dir.join(name).is_file())
}

/// Read the newer NIRUX version the bar cached, if any.
///
/// The bar checks GitHub once a day and writes `~/.cache/nirubar/nirux-update`.
/// Reading that file keeps the launcher and the bar in agreement and avoids a
/// second network request. Two version strings are compared so a stale cache
/// from an older install is ignored.
pub(crate) fn nirux_update_available() -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"))
        .join("nirubar/nirux-update");
    let latest = std::fs::read_to_string(&cache).ok()?.trim().to_string();
    let installed = installed_nirux_version()?;
    is_newer_version(&latest, &installed).then(|| {
        // The answer is worth showing once. Keeping it would offer the same
        // update after it has been installed, until the bar checks again.
        let _ = std::fs::remove_file(&cache);
        latest
    })
}

/// The active generation directory ends in the version.
pub(crate) fn installed_nirux_version() -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let target = std::fs::read_link(home.join(".local/share/nirux/current")).ok()?;
    let name = target.file_name()?.to_string_lossy().to_string();
    let version = name.split('-').next()?.to_string();
    (version.split('.').count() >= 2
        && version
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
    .then_some(version)
}

/// Compare dotted versions numerically, treating a missing part as zero.
pub(crate) fn is_newer_version(candidate: &str, installed: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(installed));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Run `niructl action list --json` with a bounded wait and bounded output.
/// The error string explains why the helper could not provide a list; normal
/// loading ignores it so an older or missing helper simply yields no rows.
pub(crate) fn load_saved_actions_result() -> std::result::Result<Vec<saved::SavedAction>, String> {
    let mut child = match Command::new("niructl")
        .args(["action", "list", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("niructl was not found".into());
        }
        Err(error) => return Err(format!("could not start niructl: {error}")),
    };
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut timed_out = false;
    let mut success = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                success = status.success();
                break;
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            _ => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
        }
    }
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout
            .take(saved::MAX_JSON_BYTES as u64 + 1)
            .read_to_end(&mut bytes);
    }
    let _ = child.wait();
    if timed_out {
        return Err("niructl action list timed out".into());
    }
    if !success {
        return Err("this niructl has no working action command".into());
    }
    Ok(saved::parse(&bytes))
}

pub(crate) fn load_saved_actions() -> Vec<saved::SavedAction> {
    load_saved_actions_result().unwrap_or_default()
}

#[cfg(test)]
mod nirux_update_tests {
    use crate::{installed_nirux_version, is_newer_version, nirux_update_available};


    struct Sandbox {
        dir: std::path::PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let guard = crate::test_env_lock();
            let dir = std::env::temp_dir().join(format!("nl-nux-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("HOME", &dir);
            std::env::set_var("XDG_CACHE_HOME", dir.join(".cache"));
            Self { dir, _guard: guard }
        }

        fn install(&self, version: &str) {
            let gen = self
                .dir
                .join(format!(".local/share/nirux/generations/{version}-X"));
            std::fs::create_dir_all(&gen).unwrap();
            std::os::unix::fs::symlink(&gen, self.dir.join(".local/share/nirux/current")).unwrap();
        }

        fn cache(&self, latest: &str) {
            let cache = self.dir.join(".cache/nirubar");
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(cache.join("nirux-update"), latest).unwrap();
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer_version("0.12.2", "0.12.1"));
        assert!(is_newer_version("0.13.0", "0.12.99"));
        assert!(!is_newer_version("0.12.1", "0.12.1"));
        assert!(!is_newer_version("0.9.0", "0.10.0"));
    }

    #[test]
    fn the_installed_version_comes_from_the_generation() {
        let box_ = Sandbox::new("installed");
        box_.install("0.12.1");
        assert_eq!(installed_nirux_version().as_deref(), Some("0.12.1"));
    }

    #[test]
    fn a_newer_cached_release_offers_the_action() {
        let box_ = Sandbox::new("newer");
        box_.install("0.12.1");
        box_.cache("0.12.2");
        assert_eq!(nirux_update_available().as_deref(), Some("0.12.2"));
    }

    #[test]
    fn an_up_to_date_machine_has_no_action() {
        let box_ = Sandbox::new("current");
        box_.install("0.12.2");
        box_.cache("0.12.2");
        assert_eq!(nirux_update_available(), None);
    }

    #[test]
    fn a_machine_without_nirux_has_no_action() {
        let _box = Sandbox::new("nosuite");
        assert_eq!(nirux_update_available(), None);
    }
}
