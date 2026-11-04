# Agent assignment: NIRULAUNCHER 0.3.0, a one-shot speed, ranking and maintainability update

## 0. Quick start

1. Read this file, `README.md`, `docs/ALIASES.md`, `src/main.rs` (≈7 000 lines) and
   `tests/ui-smoke.sh`. The audit baseline is `209d736` (0.2.78, pinned by NIRUX 0.12.199).
   Revalidate each item against current HEAD and skip items that are already fixed,
   noting that you did.
2. Implement **N1–N6, then U1–U5, and R1 last**. Commit per item. Keep `cargo fmt --check`,
   `clippy -D warnings`, tests, `tests/ui-smoke.sh` and `tests/theme-colors.sh` green.
3. Pass the gates in §4, release in §5 and report in §6.

This is an **execution** assignment. Do not stop after a plan or a branch. Make sensible
decisions, document them, and continue without routine questions.

## 1. Invariants

- One process per invocation: no daemon, no index service, and no network unless the user
  explicitly runs a web search.
- Keep the lock handoff contract (`_NIRULANCHER_SELECTION`, `_NIRULANCHER_LOCK`) from
  0.2.78 and its behavior: close promptly when NIRULOCK asks.
- Keep the safety properties: argv-only execution, no shell interpolation of queries, and
  confirmed session actions. Deterministic actions keep working without OpenCode.
- Every mode, alias, keybinding and config key keeps its meaning (`docs/ALIASES.md`,
  `config.example.toml`).
- Keep `rust-version = 1.75` and `--locked` builds on 1.75 and 1.85.

## 2. Findings (verified 2026-11-04 on niruxwork, in a separate Xvfb)

### N1: Usage is recorded but never used for ranking
`record_usage` writes `~/.local/state/nirulauncher/usage.tsv`, which had 76 rows on the
owner's machine. But `filtered()` (`src/main.rs` ~3722) takes `_usage` and ignores it, so
an app launched every day ranks the same as one never used.
**Fix:** add a frecency component:
- recent use within days ranks higher, decaying over weeks,
- used as a tie-breaker *within* the same `term_rank` bucket while searching, so a better
  text match always wins,
- and with an empty query, show "Recent" (up to 5) after the pinned favorites, then the
  alphabetical list.

Cap the file at about 500 rows (the existing pruning may already do this; verify it). Add
an option `rank_by_usage = true`.

### N2: Some views block before the first paint
Measured from process start to the window existing: `--apps` ~110 ms, but `--actions`
**~540 ms** with **42** child processes (13× niructl, xrandr, wpctl, …) and `--health`
**~640 ms** with **61** child processes, all before the window appears.
**Fix:**
- Map and paint the window with the static catalog first. Fill provider-dependent rows
  (display profiles, audio, power profile, health rows) asynchronously on the existing
  bounded provider pattern from F05. Show "…" placeholders.
- The first paint must arrive within **150 ms** in every mode, and Esc must work
  immediately.
- Remove duplicate provider calls within one open: the same `niructl` or `wpctl` query
  more than once, for example 5× `wpctl status` in `--health`. Memoize per invocation.

**Done when:** a test measures the time to map under Xvfb with fake providers that sleep
2 s, and gets ≤ 150 ms. Native numbers before and after for every mode go in the report.

### N3: Dead keys cannot be typed in the search
`is_text_key` accepts only keysyms 0x20–0xff, so `´ ^ ~ ¨` (dead keys on Swedish layouts)
are dropped, and `é`, `ü` and the like cannot be typed.
**Fix:** a minimal compose step: dead key + space or the same dead key gives the literal;
dead key + vowel gives the Latin-1 composed letter. Also make matching
**diacritic-insensitive** (`e` matches `é`, `a` matches `å`/`ä`), but rank exact matches
first. This is a search box, not a password field, so the folding is safe here.

### N4: Text input refetches the keymap
`key_text` calls `get_keyboard_mapping` per key. **Fix:** cache the map and refresh it on
`MappingNotify`. Keep the existing tested `keyboard_level`/`apply_caps` helpers.

### N5: The monolith
`src/main.rs` is about 7 000 lines with about 90 top-level items: modes, providers,
rendering, input and tests. This is addressed by R1 at the end.

### N6: `sudo -n` on every Wi-Fi open
`--wifi` runs `sudo -n /usr/libexec/nirux-wifi …` 2–3 times per open. That is by design
(NOPASSWD rule), but each call writes to the auth log. Verify that the results are
memoized per open (no duplicate `status` and `list_networks`), and batch them into one
helper call if the helper supports it. Do not change the helper or the sudoers rule.

## 3. UX

### U1: Show why a row is there
While searching, show the matched part of the name in bold or underlined, so it is clear
why a row matched, especially subsequence matches. Rows that matched only through
keywords or detail show the matching keyword dimmed after the name.

### U2: Calculator and unit row
When the query parses as arithmetic (`2+2*3`, `(1920/1080)`, `15% of 240`), show one
**=** result row at the top. Enter copies the result to the clipboard through `xclip`,
which is already a suite dependency. Write a pure-Rust parser with no new crates and
bounded input length. Never evaluate anything else, and never run a shell.

### U3: Empty state and hints
With no results, show "No matches. Press Super+S to search the web for ‘query’", plus
"Tab: switch view" hints. With an empty query, show a one-line hint for the current mode.

### U4: Keyboard paging
Add PageUp/PageDown/Home/End in all list modes, and Ctrl+Backspace to delete a word.

### R1: Split `main.rs` into modules (last, behavior-neutral)
After everything above is green, move code into modules **without changing behavior**:
`modes/` (one file per mode family), `providers/`, `input.rs`, `rank.rs`, `ui.rs` and
`app.rs`.
- Do it in mechanical commits that only move code, then separate cleanup commits.
- Tests move with their code.
- `main.rs` ends up at ≤ 400 lines of orchestration.
- The binary's behavior, CLI and output stay identical: `tests/ui-smoke.sh` and all unit
  tests pass after every commit.

Out of scope: a plugin system, network indexing, clipboard history, and new modes.

### U5: Event and per-app notification sounds in the Sound view
NIRUX `main` (`ef7a500`, shipped in the first suite release after 0.12.202) adds per-app
notification sounds and a machine-readable status. The Sound view (`--sound`) has no
event-sound section today. Add one that uses only `nirux sounds`. It owns no config of
its own and parses no TOML:
- Read the state with `nirux sounds status --json`, schema 1:
  `{"enabled","volume","config","events":{startup,notification,update,shutdown},
  "exclude":[…],"apps":[{"app","sound","state":"ok|missing|silent|default","file"}]}`.
  Run it once per open on the bounded provider pattern (N2), and treat a missing `nirux`
  or an older NIRUX without `--json` as "Event sounds unavailable".
- Rows:
  - **Event sounds on/off** (`nirux sounds on|off`).
  - One toggle per event (`nirux sounds toggle EVENT`).
  - **Test** per event (`nirux sounds test EVENT`).
  - Per app: "Thunderbird → mail" or "Signal → chat", with the state shown as ok,
    missing (falls back to the notification sound), silent or default.
  - Per-app actions: choose a sound (a list built from the `*.ogg` names in the NIRUX
    sounds dir and `~/.config/nirux/sounds/`, plus *Default* and *Off*), **Test**
    (`nirux sounds test notification --app APP`) and **Remove**
    (`nirux sounds app APP remove`).
  - **Add app…**: suggest app names seen in `nirunotify --history` (column 2, unique,
    most recent first) that have no entry yet, and also accept free text. Validate with
    the same rules as `nirux sounds app`: letters, digits, `.`, `_` and `-`.
- Run every change with argv only (`nirux sounds app APP SOUND`), off the UI thread, then
  re-read the JSON. Never run a shell.
- Volume is display-only here unless `nirux` gains a setter. Do not edit `sounds.toml`.

**Done when:** a fake `nirux` on PATH (JSON fixture + an argv log) drives tests for
rendering, every action's argv, an older NIRUX without `--json`, and invalid app names.
Natively on niruxwork, Thunderbird and Signal show their sounds, Test plays them, and
changes show up in `nirux sounds status`. Restore the owner's mappings afterwards.

## 4. Gates

Automated:
- fmt, clippy `-D warnings`, `cargo test --locked`, a release build,
  `tests/ui-smoke.sh`, `tests/theme-colors.sh`, and an MSRV job on 1.75.
- New tests:
  - frecency ordering, including the rule that a better text match beats more frequent
    use,
  - recent items in the empty query,
  - the compose and diacritic folding table,
  - the calculator parser, including rejection of non-arithmetic and length/overflow
    bounds,
  - the N2 first-paint budget with slow fake providers,
  - memoization: each provider is called at most once per open, counted with a PATH shim.

Native, on **niruxwork** (the owner allows local testing):
- Before and after: the open time and child process count for `--apps`, `--actions`,
  `--settings`, `--sound`, `--wifi`, `--health` and `--displays`.
- A Swedish-layout search with `é`.
- Ranking after a few launches.
- A calculator copy.
- A lock handoff smoke test (launcher open → `niructl lock` on an authorized host).

Do not change the owner's `favorites.tsv`/`usage.tsv`. Use a temporary `XDG_STATE_HOME` for
tests.

## 5. Release

Bump to **0.3.0**. Tag `v0.3.0` through the normal workflow and verify the assets and
checksums. Then pin it in `nirucon/nirux` `manifest`:
- If `nirux/tools/suite-pin` exists, use it.
- Otherwise bump the manifest the usual way. Change only the nirulauncher line, and rebase
  if other pins land at the same time.

Install through `nirux update` and verify the running version.

## 6. Final report

For each item N1–N6, U1–U5 and R1: the change, the tests and the evidence (open times and
spawn counts per mode before and after, `main.rs` line count before and after). Include
release URLs, installed versions and remaining limits. Mark anything unfinished as
**PARTIAL** and give the exact remaining steps.
