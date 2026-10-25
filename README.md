# NIRULAUNCHER

**Part of [NIRUX - A minimal X11 desktop by Nicklas Rudolfsson](https://github.com/nirucon/nirux).**

**Early beta.** Keyboard-first application, window and desktop-action launcher
for the NIRUX X11 desktop. One process per invocation: no daemon, indexing service or network.


## Install

Use [NIRUX](https://github.com/nirucon/nirux#install) for the complete desktop,
or install independently:

```sh
git clone https://github.com/nirucon/nirulauncher
cd nirulauncher
./install.sh
./install.sh --build
```

Requires an X11 session and a monospaced font through fontconfig or
`font_path`. Release assets target x86_64/glibc.

## Use

```sh
nirulauncher --apps
nirulauncher --search             # type a phrase, Enter opens a search engine
nirulauncher --search PHRASE      # the phrase is already in the field
nirulauncher --list-search        # the engines and their addresses
nirulauncher --actions
nirulauncher --windows
nirulauncher --microphone
nirulauncher --bluetooth
nirulauncher --displays
nirulauncher --power
nirulauncher --check-config
```

Super+P opens application search. Super+Shift+P opens applications, windows and
grouped actions. `--windows` switches directly to an open window.

`--actions` groups System, Display, Power, Appearance, Wallpaper, Workspaces
and Windows. The Appearance group includes theme selection and a compositor
toggle for picom transparency and shadows. The Wallpaper group has one entry
that opens **NIRU Wallpaper**, where an image, its scale, rotation and the
screen are chosen with previews; the launcher no longer repeats a list of
images or single settings. Layouts are listed with their shortcuts: Tile, Left stack, Monocle,
Centered, Centered floating, Floating and Fibonacci, plus previous/next.
The list is alphabetical, with up to five pinned favourites first. Press
Ctrl+P on a row to pin or unpin it.
It exposes audio sources and outputs, microphone mute/gain, Bluetooth device
actions, display-profile preview/keep/revert, power profiles, diagnostics, work
modes, recipes, lock, reload, DND, idle inhibition, brightness, night light,
themes, workspaces and confirmed session actions.

### Web search

`--search` turns the launcher into an address bar. From three characters on,
every configured search engine appears as its own row at the top of the list,
one per line with the address it opens; the local application matches stay
below them, so the launcher is still a launcher. Enter opens the selected
engine, Tab or the arrow keys move between engines, and Escape closes.

Ten engines are built in, ready to use:

| id | Engine |
|---|---|
| `duckduckgo` | DuckDuckGo |
| `brave` | Brave Search |
| `google` | Google |
| `wikipedia` | Wikipedia (English) |
| `youtube` | YouTube |
| `chatgpt` | ChatGPT |
| `gemini` | Gemini |
| `grok` | Grok |
| `copilot` | Microsoft Copilot |
| `metal-archives` | Metal Archives |

Adding one is a few lines in `~/.config/nirulauncher/config.toml`; no rebuild
and no new mode:

```toml
[[search.sources]]
id = "searx"
label = "SearXNG (home)"
url = "https://searx.example/search?q={query}"
```

`{query}` is replaced by the percent-encoded phrase. An id that matches a
built-in replaces it in place (so the familiar engines keep their order); a new
id is appended after them. `enabled = false` turns an engine off. Only `http`
and `https` addresses are opened, so a mistyped template cannot start a local
program.

The launcher never contacts the network: it hands the address to the browser
named by `browser` in the launcher config, else NIRUWM's `browser`, else
`$BROWSER`, else the desktop default through `xdg-open`.

Desktop `Exec` lines are parsed into argv and never passed through a shell.
Applications with `Terminal=true` use the configured terminal argv. Favourites
are configured in `~/.config/nirulauncher/config.toml`; launch recency is stored
under `~/.local/state/nirulauncher/`. Colours and font come from
`~/.config/niruwm/config.toml`.

Current limits: no input-method framework, English `Name` keys, and Latin-1-only
keysym text. Shift, Caps Lock and AltGr are handled, including Swedish letters.

## NIRUX suite

[NIRUX](https://github.com/nirucon/nirux) · [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot)

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -- --test-threads=1
sh tests/ui-smoke.sh
```

MIT. Copyright 2026 Nicklas Rudolfsson.
