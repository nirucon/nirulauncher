# Changelog

## 0.14.0 (2026-11-12)

- **Workspace overview** (`--overview`, Super+E in NIRUWM): one card
  per tag on every monitor, with the thumbnail NIRUWM saved, the tag number
  and the window titles. Number keys pick a tag on the focused monitor,
  arrows, Tab, Home and End move, Enter or a click goes there through
  `niructl do goto`, Escape or a click outside closes. Missing pictures show
  a plain card; a missing or newer `state.json` shows a short message.
  `--overview` no longer opens the window switcher; use `--windows`.
- **`--apps --query TEXT --launch-best`** for NIRUDICTATE's voice commands:
  starts the app the text clearly means (`launched<TAB>Name`) or opens the
  Apps view with the text typed (`shown`). The rule is in the README.
  `--apps --query TEXT` alone opens the Apps view with the text typed.
- **`--note --stdin`** saves standard input to the inbox like the quick
  note, without a window, and prints the inbox path; an empty note exits 1.

## 0.13.1 (2026-11-12)

- The window jump labels show their letters. The drawing was never sent to
  the X server, so the labels stayed plain boxes.

## 0.13.0 (2026-11-12)

- **Window jump** (`--hints`, Ctrl+Alt+W in NIRUWM 0.25.2): every visible
  window on every screen gets a label; typing it focuses that window and
  moves the pointer there. Letters (home row first) or numbers with
  `[hints] style`. Labels never share a prefix, so there is no waiting.
- **Quick note** (`--note`, Super+Shift+N): one line, added to the inbox
  file (`[note] file`, `~/Notes/inbox.md`) with the time. The rows below
  choose whether it also goes to a Discord webhook or a trusted device
  through NIRUSHARE.
- **Quick Settings: battery charge limit.** *Charge to 100 % once* or *Back
  to the charge limit*, or why this laptop has no limit (NIRUWM 0.25.2
  `niructl battery`).

## 0.12.1 (2026-11-12)

0.12.0 was tagged but not released (formatting check); 0.12.1 is the same with the code formatted.

- **Settings history in NIRU Actions** (`Super+A`, Appearance): *Undo the
  last settings change* (asks first, runs `niructl config undo`), *Settings
  history* and *Settings that differ from the defaults* (in a terminal that
  stays open). Needs NIRUWM 0.25.1.
- *Reset bar configuration* now only runs `nirubar --reset-config`, which
  keeps the old file in the settings history and reloads the bars itself.
- Tests that change `HOME` or the XDG folders share one lock, and the manual
  tests use a folder each; they no longer fail now and then when run in
  parallel.

## 0.11.0 (2026-11-10)

- **Microphone inputs of a multichannel interface** in the Capture view
  (`--clip`), for NIRUCLIP 0.7.0 and newer. When `niruclip --list-mics --json`
  reports a source with more than two channels (a Behringer UMC1820 has 18),
  a row *Mic inputs · CHOICE* appears for it. Enter opens a picker: all
  inputs, each input alone as mono, or the adjacent pairs 1+2, 3+4 and so on
  as stereo. When the interface is not the microphone NIRUCLIP would use,
  the picker also offers *Not used*, and choosing inputs records from the
  interface (`--mic NAME`). The choice is remembered per source in
  `~/.local/state/nirulauncher/clip-inputs.tsv`, never in NIRUCLIP's config.
- The recording rows (full screen, region, webcam, camera corners) then pass
  `--mic-channels N`, `N,M` or `all`. A profile gets `--mic-channels` when its
  microphone is the interface and it does not set `mic_channels` itself; its
  row shows the inputs. Profiles with desktop audio or no audio are unchanged.
- The microphones are read in the worker that already reads the profiles, so
  the view paints at once. Enter on a recording row before they are read
  waits for them, but only after a choice has been made.
- Without a multichannel source, or with an older NIRUCLIP, the view is
  exactly as before.

## 0.10.0 (2026-11-09)

- **Manual view.** `nirulauncher --manual`, *Manual* in NIRU Actions and the
  intent phrases `manual`, `help`, `hjälp`, `handbok`, `läs manualen` list the
  chapters and headings of the NIRUX manual installed with the release (from
  `nirux manual --path`). Typing searches headings and the text under them;
  Enter runs `nirux manual CHAPTER#ANCHOR`, which opens the chapter at that
  heading. Without an installed manual (NIRUX 0.12.245 and older) the view
  shows one row saying so.

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
