# NIRU Launcher

Keyboard-first launcher for applications, windows, web search, settings and NIRU Actions on the NIRUX desktop.

**Status: beta** · Part of the [NIRUX](https://github.com/nirucon/nirux) suite · [MIT License](LICENSE)

![NIRU Launcher](docs/screenshots/main.png)

## About

NIRU Launcher (`nirulauncher`) is the one keyboard entry point to NIRUX. It
starts applications, switches windows, searches the web, opens Quick Settings
and runs NIRU Actions: practical system actions such as display profiles,
power, Wi-Fi, Bluetooth, sound, themes and diagnostics.

It is one process per invocation: no daemon, no indexing service and no network
access of its own. Settings views are thin front ends over the same `niructl`,
`nirux` and system providers the rest of the suite uses, so there is never a
second, competing settings implementation.

## Features

- Application search with multi-word, keyword, diacritic-insensitive and
  conservative fuzzy matching, favourites, pins and launch-history ranking.
- Window switcher and workspace-grouped overview.
- Dedicated web search with seventeen built-in engines, aliases (`gh niruwm`)
  and your own engines in a few lines of TOML.
- NIRU Actions: a grouped catalogue of system, display, power, appearance,
  wallpaper, Wi-Fi, workspace and window actions, plus saved actions.
- Deterministic intent phrases ("lock", "focus mode", "volume up") in English,
  Swedish and more, without any model or network.
- Settings views: Wi-Fi, wired Ethernet, Bluetooth, sound and event sounds,
  microphone, displays, power, themes, timing, input, key bindings and Health.
- Calculator row (`2+2*3`, `15% of 240`), copied with Enter.
- Optional OpenCode handoff through `niructl agent`, never auto-approved.
- HiDPI scaling from `Xft.dpi`, dead-key composition and AltGr levels.

## Requirements

- Debian, Arch or Void Linux (glibc), x86_64, with X11.
- A monospaced font through fontconfig, or `font_path`.
- Optional: `niructl` (NIRUWM) for actions and settings, NetworkManager or
  `wpa_supplicant` for Wi-Fi, `xclip` for the calculator, `opencode` for the
  agent handoff.
- Building from source: Rust and `cargo`.

## Installation

### With NIRUX

NIRU Launcher is a core component. `nirux install` installs it with the rest of
the suite and `nirux update` keeps it current. See the
[NIRUX README](https://github.com/nirucon/nirux#installation).

### Standalone

```sh
git clone https://github.com/nirucon/nirulauncher
cd nirulauncher
./install.sh            # download the released binary and verify its checksum
./install.sh --build    # or build from this checkout (needs cargo)
```

The binary goes to `~/.local/bin` (override with `NIRULAUNCHER_PREFIX`) and the
example configuration to `~/.config/nirulauncher/` when none exists. The
installer refuses to replace a NIRUX-managed copy; use `nirux update` there.

## Usage

Default NIRUWM keys:

| Keys | View |
|---|---|
| `Super+P` | Applications |
| `Super+Shift+P` | Applications, windows, actions and settings in one view |
| `Super+A` | NIRU Actions |
| `Super+S` | Web search |
| `Super+V` | Clipboard history |
| `Alt+Tab`, `Super+W`, `Super+grave` | Window switcher |

Inside the launcher: ↑/↓ and Tab move, PageUp/PageDown page, Home/End jump,
Enter opens, Escape closes, Ctrl+Backspace deletes a word, Ctrl+P pins or
unpins a row and Ctrl+S saves an app or command row as a NIRU Action.

Each view can be opened directly:

| Option | View |
|---|---|
| `--apps` | Application search (default) |
| `--actions` / `--system-actions` | Grouped NIRU Actions / the focused actions view |
| `--windows` (alias `--overview`) | Window switcher |
| `--search [PHRASE]` | Web search, optionally with the phrase filled in |
| `--quick` | Quick Settings |
| `--clipboard` | Clipboard history |
| `--settings` | NIRUX Settings: appearance, wallpaper, displays, audio, keys, timing |
| `--themes`, `--displays`, `--power`, `--timing`, `--input`, `--keys` | Single settings views |
| `--wifi`, `--wired`, `--bluetooth`, `--tailscale` | Network and devices |
| `--sound`, `--microphone` | Outputs, inputs, streams, event sounds, microphone test |
| `--clip` | NIRUCLIP capture profiles, overlays, recovery and status |
| `--share`, `--share-pair` | NIRUSHARE devices, inbox, pairing and Discord webhooks |
| `--health` | Installed/running, session, audio, network, display and capture status |
| `--agent` | Short OpenCode prompt, then a centred terminal |

Non-interactive helpers: `--list`, `--list-actions`, `--list-search`,
`--wifi-status`, `--wifi-scan`, `--wired-status`, `--check-config` and
`--version`.

**Share and Discord.** With NIRUSHARE 0.4.0 or newer, the Share view lists
every Discord webhook configured in `~/.config/nirushare/webhooks.toml` as
*Share clipboard to Discord · NAME* (or *Share image …* when NIRUSHOT's review
opened the view with a screenshot). Choosing one never sends anything: it
opens a preview view that shows the target, the content, its size, how many
messages it becomes and the start of the text. **Send to Discord · NAME**
posts exactly what the preview showed; **Cancel** goes back to Share. The
"Sent to Discord" notification has **Undo** for 5 minutes. The launcher only
ever sees webhook names, never their URLs. With an older NIRUSHARE the
Discord rows simply do not appear.

The complete behaviour of every view (actions catalogue, intent phrases,
Wi-Fi, wired networking, web search, sound and settings) is described in
[docs/GUIDE.md](docs/GUIDE.md).

## Configuration

Launcher settings live in `~/.config/nirulauncher/config.toml`; start from
[config.example.toml](config.example.toml). Colours and font come from the
NIRUWM theme in `~/.config/niruwm/config.toml`.

```toml
width = 640
max_rows = 12
row_height = 26
padding = 12
font_size = 15.0
terminal = ["kitty"]          # for Terminal=true applications
# browser = ["brave"]         # else NIRUWM's browser, $BROWSER, xdg-open
favorites = ["firefox", "kitty"]
rank_by_usage = true

[search]
default = "duckduckgo"
order = ["duckduckgo", "github", "archwiki", "wikipedia"]

[[search.sources]]
id = "searx"
label = "SearXNG (home)"
url = "https://searx.example/search?q={query}"
alias = "sx"
```

`{query}` is replaced by the percent-encoded phrase; only `http` and `https`
addresses are opened. `enabled = false` on a built-in id turns that engine off.
Set `NIRU_UI_SCALE` (0.75–2.5) to override HiDPI detection, or `1` to disable
it. Check a file with `nirulauncher --check-config`.

## Files and paths

| Path | Purpose |
|---|---|
| `~/.config/nirulauncher/config.toml` | Launcher settings |
| `~/.local/state/nirulauncher/usage.tsv` | Launch history (last 500 launches) |
| `~/.config/niruwm/config.toml` | Theme, font and browser fallback |
| `~/.local/bin/nirulauncher` | Program |

## Troubleshooting

- **A settings view says "unavailable".** The provider is missing or too old
  (for example `niructl`, `nmcli` or an older NIRUX); the launcher never
  pretends an action succeeded.
- **Wi-Fi cannot connect through iwd or ConnMan.** They are detected but not
  controlled; use NetworkManager or `wpa_supplicant`.
- **Text is tiny or huge.** Set `NIRU_UI_SCALE`.
- **A web search engine opens the wrong browser.** Set `browser` in the
  launcher config.

## Uninstall

With NIRUX, use `nirux uninstall`. A standalone install is removed with:

```sh
./install.sh --uninstall
```

Configuration in `~/.config/nirulauncher` is kept.

## Building from source

```sh
cargo build --release --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -- --test-threads=1
sh tests/ui-smoke.sh target/release/nirulauncher
sh tests/theme-colors.sh target/release/nirulauncher
sh tests/first-paint.sh target/release/nirulauncher   # 150 ms first paint, memoized providers
sh tests/event-sounds.sh target/release/nirulauncher  # Sound view through a fake nirux
```

## Status, disclaimer and how it's made

**Status: beta.** I use the launcher dozens of times a day; the application,
window and search views are solid, while some settings views depend on
providers that are not available everywhere.

NIRUX and every niru* project are made by me, Nicklas Rudolfsson
([@nirucon](https://github.com/nirucon)), for me. They are tailored to my own
needs, preferences and workflows, on my own machines. If they fit yours too,
that's a nice bonus.

- **Free and open source.** Anyone is free to use, study, change and share
  it under the [MIT License](LICENSE).
- **No guarantees.** There is no warranty of any kind. Use it at your own
  risk, read what you install and keep backups of anything that matters.
- **How it's made.** Some of these projects I built from scratch by hand. In
  more recent releases I have used Claude, Anthropic's AI, to do much of the
  heavy lifting under my direction: I decide what to build and how it should
  behave, and I test it on my own machines. It saves me a lot of time, and
  the result is quite OK.
- **Alpha, beta, dev, and maybe forever.** Most things here are alpha, beta
  or dev, and may stay that way. Maybe one day there will be a real stable
  release, but given how fast development moves, and how I am as a person,
  maybe never :)

| Label | Meaning |
|---|---|
| dev | Under active construction; may not build, install or work yet. |
| alpha | Works on my machines, but incomplete or rough; expect breaking changes. |
| beta | In daily use on my machines and mostly complete; rough edges remain. |

## License

MIT License, see [LICENSE](LICENSE). Copyright (c) 2026 Nicklas Rudolfsson.
