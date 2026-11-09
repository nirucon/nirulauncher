# Changelog

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
