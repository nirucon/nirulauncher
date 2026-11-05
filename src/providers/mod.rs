//! Providers: the programs and services the views read, and the memo that
//! asks each read-only query once per launcher process.
//!
//! One open asks each provider once: the Health view and the actions catalog
//! read the same `niruclip --status --json`, and switching between views must
//! not ask `niructl` again for what it already answered. Anything that changes
//! state calls [`forget`] so the next read is fresh.

pub(crate) mod eventsound;
pub(crate) mod sound;
pub(crate) mod tailscale;
pub(crate) mod wifi;

use std::collections::HashMap;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

type Key = Vec<String>;
type Answer = Result<Output, (std::io::ErrorKind, String)>;

/// Answers, plus queries that another thread is already running.
enum Slot {
    Running(Arc<(Mutex<bool>, Condvar)>),
    Done(Answer),
}

static CACHE: Mutex<Option<HashMap<Key, Slot>>> = Mutex::new(None);

fn key(program: &str, args: &[&str]) -> Key {
    std::iter::once(program)
        .chain(args.iter().copied())
        .map(str::to_string)
        .collect()
}

/// Run `fetch` once per argv; concurrent callers wait for the first one.
fn memo(key: Key, fetch: impl FnOnce() -> Answer) -> Answer {
    let latch = {
        let mut cache = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
        let map = cache.get_or_insert_with(HashMap::new);
        match map.get(&key) {
            Some(Slot::Done(answer)) => return answer.clone(),
            Some(Slot::Running(latch)) => Some(latch.clone()),
            None => {
                map.insert(
                    key.clone(),
                    Slot::Running(Arc::new((Mutex::new(false), Condvar::new()))),
                );
                None
            }
        }
    };
    if let Some(latch) = latch {
        let (done, signal) = &*latch;
        let mut finished = done.lock().unwrap_or_else(|poison| poison.into_inner());
        while !*finished {
            finished = signal
                .wait(finished)
                .unwrap_or_else(|poison| poison.into_inner());
        }
        drop(finished);
        // The first caller stored its answer; a forget() in between simply
        // means one more query.
        return memo(key, fetch);
    }
    let answer = fetch();
    let mut cache = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(Slot::Running(latch)) = map.insert(key, Slot::Done(answer.clone())) {
        let (done, signal) = &*latch;
        *done.lock().unwrap_or_else(|poison| poison.into_inner()) = true;
        signal.notify_all();
    }
    answer
}

/// `Command::output()` for a read-only query, at most once per process.
pub fn output(program: &str, args: &[&str]) -> std::io::Result<Output> {
    memo(key(program, args), || {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| (error.kind(), error.to_string()))
    })
    .map_err(|(kind, message)| std::io::Error::new(kind, message))
}

/// A bounded query (see `bounded_output`), at most once per process. `None`
/// means the provider is missing or did not answer within `budget`.
pub fn bounded(program: &str, args: &[&str], budget: Duration) -> Option<Output> {
    memo(key(program, args), || {
        let limit = format!("{:.2}", budget.as_secs_f64());
        match Command::new("timeout")
            .args(["-k", "0.2", &limit, program])
            .args(args)
            .stdin(Stdio::null())
            .output()
        {
            Ok(output) if output.status.code() == Some(124) => {
                Err((std::io::ErrorKind::TimedOut, format!("{program} timed out")))
            }
            Ok(output) => Ok(output),
            Err(error) => Err((error.kind(), error.to_string())),
        }
    })
    .ok()
}

/// Drop every answer, after the launcher itself changed something.
pub fn forget() {
    let mut cache = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(map) = cache.as_mut() {
        map.retain(|_, slot| matches!(slot, Slot::Running(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_query_runs_once_until_forgotten() {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let fetch = || {
            RUNS.fetch_add(1, Ordering::SeqCst);
            Err((std::io::ErrorKind::NotFound, "missing".into()))
        };
        let key = vec!["memo-test".to_string(), "once".to_string()];
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let key = key.clone();
                std::thread::spawn(move || memo(key, fetch).is_err())
            })
            .collect();
        for thread in threads {
            assert!(thread.join().unwrap());
        }
        assert_eq!(
            RUNS.load(Ordering::SeqCst),
            1,
            "concurrent callers share one run"
        );
        forget();
        assert!(memo(key, fetch).is_err());
        assert_eq!(RUNS.load(Ordering::SeqCst), 2);
    }
}
