# Changelog

## 0.9.0 (2026-11-09)

- **Locks from the launcher are manual locks.** *Lock screen* in Quick
  Settings, the *lock* intent and NIRU Actions run `niructl lock --manual`.
  NIRULOCK 0.25 plays its optional pause music only for a manual lock, and a
  plain `niructl lock` without a terminal (as the launcher starts it) now
  counts as a script's lock and stays silent. An older NIRUWM ignores the
  option and locks as before.

## 0.8.0 (2026-11-09)

- **Studio mode toggle** for NIRUWM 0.20's studio (DAW) mode: Quick Settings
  and NIRU Actions (*System*) show *Studio mode: on/off*, read from the root
  property `_NIRUWM_STUDIO`, and Enter runs `niructl studio toggle`. With an
  older NIRUWM the row reads *unavailable* with a note and does nothing.
- Intent phrases `studio mode`, `toggle studio mode`, `daw mode`,
  `recording mode`, `studioläge`, `inspelningsläge` (and the forms without
  diacritics). A bare `studio` stays an ordinary search (OBS Studio,
  Android Studio).

## 0.7.0 (2026-11-09)

- **Window swallowing toggle.** Quick Settings and NIRU Actions (*System*)
  show *Window swallowing: on/off*, read from NIRUWM's `_NIRUWM_SWALLOW` root
  property without starting a process (`--list-actions` asks
  `niructl swallow status`); Enter runs `niructl swallow toggle`. With a
  NIRUWM older than 0.19 the row reads *unavailable* with a note and does
  nothing. Intent phrases: `window swallowing`, `swallow`,
  `fönstersväljning`, `svälj fönster`.
- **File search hint:** "/ for files" is no longer shown in the apps and
  actions hints when `[files] enabled = false` (the `/` prefix already stayed
  ordinary text then; a test now covers it). `--files` while turned off says
  so in its hint.
- **New optional `[files] locate_db`:** search a plocate/locate database you
  built yourself (`-d`), e.g. `updatedb.plocate -o ~/.cache/files.db -U ~ -l no`
  without root. A configured database that is missing is reported and the
  next backend is used, never the system database in its place.
- **Fix:** a search root whose path contains `[`, `*` or `?` (`~/Musik [2024]`)
  was never found in a locate database, because locate treats such a plain
  pattern as a whole-path glob, so the launcher fell back to fd. The coverage
  check now uses an anchored regex for such roots.
- **Verified with the real plocate 1.1.23** (Debian trixie package, unpacked
  without root) and a user database from `updatedb.plocate -U HOME -l no`:
  `-d`, `-0`, `-i`, `-l N`, `--regex` with several ANDed patterns, the
  accent-alternation patterns (`faktura aao` finds `faktura-åäö.pdf`,
  `lat ett rpp` finds `Låt ett.RPP`), regex characters in queries (`c++`,
  `a.b`), names starting with `-`, hidden and excluded folders filtered,
  files deleted since `updatedb` left out, and the coverage check. Note that
  `LOCATE_PATH` alone does not work when the system database is missing
  (plocate still opens the default first), hence `locate_db`.
  `tests/files.sh` now runs these checks with a real plocate when one is
  installed, and CI installs it.

## 0.6.0 (2026-11-09)

- **File search** (`--files`, *Find files* in NIRU Actions, or `/ name` in
  the unified view): the home folder without hidden folders, caches, the
  trash and build output (`.git`, `node_modules`, `target`, `__pycache__`,
  `.venv`, …), with recently used files (`recently-used.xbel`) first and
  listed when the field is empty. Multi-word, case- and diacritic-insensitive
  (`faktura aao` finds `faktura-åäö.pdf`); a word in the name ranks above one
  in a folder. Rows show the kind, the name and the folder (`~/Fakturor`).
  Enter opens with `xdg-open`, Ctrl+Enter opens the folder, Shift+Enter or
  Ctrl+C copies the path.
- Backends: `plocate` or mlocate when the database covers the roots, else
  `fd`/`fdfind`, else a built-in walker that lists the roots once per open
  (bounded to 250 000 entries and 8 s). Searches run in a worker, are
  cancelled by the next key and stop after 1.5 s; the footer names the
  backend and the time.
- New optional `[files]` section: `enabled`, `roots`, `exclude`, `backend`,
  `max_results`, `show_hidden`. Existing configurations need no change.
- `--list-files WORDS` prints what the search finds.
- New test `tests/files.sh` (every backend, recent files, Enter, Ctrl+Enter,
  Shift+Enter, the `/` prefix) in CI and the release workflow.
- install.sh installs this release by default.

## 0.5.1 (2026-11-08)

- **Capture view (`--clip`) for NIRUCLIP 0.4.0:** *Check camera*
  (`niruclip --check-camera`: privacy key/shutter, frozen picture, camera in
  use), *Camera preview* (a live window with the overlay shape), *Last
  recording → MP4/GIF* and *Share last recording* (MP4 through NIRU Share).
  A running recording with a camera problem shows a *Camera problem* row with
  NIRUCLIP's message. The status is read once per view instead of twice.
- NIRU Share names a pending video file a *recording* instead of an *image*.

## 0.5.0 (2026-11-08)

- **Keybinding cheat sheet** (`--keys`, NIRUWM's new `Super+/`). Rows come
  from `niructl keys --json` (NIRUWM 0.18.0): an English description for
  every action, your own `[[keys]] description`, and the keys other programs
  register in `~/.local/share/niru/keys.d`. Grouped with headings; search by
  words or by key (`super+t`, `ctrl alt space`); Enter runs the binding
  (`niructl keys run`) or a registered program's command; Ctrl+R shows what a
  pressed combination does or that it is free; clashes are marked *conflict*
  and the footer names the other program. An older niructl still gives the
  plain list.
- **Emoji and symbol picker** (`--emoji`, NIRUWM's new `Super+.`): Unicode
  16.0 emoji and 85 text symbols (©, ®, ™, arrows, °, ±, currency signs,
  typographic quotes) with English and Swedish CLDR names and keywords,
  compiled in (`data/emoji.tsv`, Unicode License v3). Color emoji are drawn
  from Noto Color Emoji's CBDT bitmaps (ttf-parser, png), shaped with the
  font's own ligatures, scaled once and cached; the font is memory-mapped and
  only opened when the first emoji row is drawn. Recents first; Tab and
  Shift+Tab cycle the skin tone (remembered). Enter pastes into the previous
  window and then restores the clipboard with every format it held;
  Shift+Enter only copies. `:name` in the unified view offers the best eight.
- **Colors view** (`--colors`, also *Colors* in NIRU Actions): the last ten
  colors from `~/.local/state/nirushot/colors.json` with swatches; Enter
  copies the hex. *Pick color* runs `nirushot pick` (NIRUSHOT 0.6.0).
- Characters the monospaced font lacks fall back to DejaVu Sans.
- New tests: `tests/pickers.sh` (emoji copy, tone, recents, Colors, cheat
  sheet Enter) in CI and the release workflow; first-paint covers the new
  views.
- install.sh installs this release by default.

## 0.4.0 (2026-11-07)

- Share view: one row per Discord webhook configured in NIRUSHARE
  (`nirushare discord list --json`, names only), *Share clipboard to Discord ·
  NAME*, or *Share image …* when the NIRUSHOT review opened the view.
- New preview view `share-discord:NAME`: it stages the content with
  `nirushare discord preview --json` and shows the target, content, size,
  number of messages and the start of the text. **Send** runs
  `nirushare discord confirm ID`, so exactly the previewed content is posted;
  **Cancel** returns to Share. Nothing is sent from the list itself.
- Needs NIRUSHARE 0.4.0 for the Discord rows; with an older NIRUSHARE they do
  not appear and everything else works as before.
- install.sh installs this release by default (it still named 0.2.45).
