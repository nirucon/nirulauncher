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
nirulauncher --search             # dedicated web search, every engine listed
nirulauncher --search PHRASE      # the phrase is already in the field
nirulauncher --list-search        # the engines and their addresses
nirulauncher --actions
nirulauncher --windows
nirulauncher --microphone
nirulauncher --bluetooth
nirulauncher --displays
nirulauncher --power
nirulauncher --keys               # active keybindings, including overrides
nirulauncher --settings           # appearance, wallpaper, displays, audio, keys, timing
nirulauncher --themes             # theme selection
nirulauncher --timing             # idle, lock and display-off timers
nirulauncher --check-config
```

Super+P opens application search. Super+Shift+P opens applications, windows and
grouped actions. Super+S opens the dedicated web search. `--windows` switches
directly to an open window.

`--actions` groups System, Display, Power, Appearance, Wallpaper, Workspaces,
Windows and NIRUX Settings. The Appearance group includes theme selection and a
compositor toggle for picom transparency and shadows. The Wallpaper group has
one entry that opens **NIRU Wallpaper**, where an image, its scale, rotation and
the screen are chosen with previews; the launcher no longer repeats a list of
images or single settings. Layouts are listed with their shortcuts: Tile, Left stack, Monocle,
Centered, Centered floating, Floating and Fibonacci, plus previous/next.
The list is alphabetical, with up to five pinned favourites first. Press
Ctrl+P on a row to pin or unpin it.
It exposes audio sources and outputs, microphone mute/gain, Bluetooth device
actions, display profiles (preview/keep/revert, arrange side by side, and a
"turn this screen" entry for every rotation), a link to **NIRU Display** when it
is installed, power profiles, diagnostics, work
modes, recipes, lock, reload, DND, idle inhibition, brightness, night light,
themes, workspaces and confirmed session actions.

### Web search

`--search` is a dedicated address bar. Opening it lists every enabled engine
immediately, with no application rows at all; type one character and every row
becomes a ready-to-open search. Enter on an empty field keeps the launcher open
and asks for a phrase. The first row becomes **Open address** when the text is
an explicit `http://` or `https://` URL.

Arrow keys/Tab move between engines, Enter opens the selected one and Escape
closes. The selected engine stays selected while the phrase is edited, so the
same keystrokes always open the same engine.

Seventeen engines are built in:

| id | Engine | Alias |
|---|---|---|
| `duckduckgo` | DuckDuckGo | `ddg` |
| `brave` | Brave Search | `brave` |
| `google` | Google | `g` |
| `wikipedia` | Wikipedia (English) | `w` |
| `wikipedia-sv` | Wikipedia (Swedish) | `wsv` |
| `youtube` | YouTube | `yt` |
| `github` | GitHub | `gh` |
| `archwiki` | ArchWiki | `aw` |
| `crates` | crates.io | `crate` |
| `docsrs` | docs.rs | `docs` |
| `chatgpt` | ChatGPT | `gpt` |
| `gemini` | Gemini | `gem` |
| `grok` | Grok | `grok` |
| `copilot` | Microsoft Copilot | `cop` |
| `discogs` | Discogs | `disc` |
| `musicbrainz` | MusicBrainz | `mb` |
| `metal-archives` | Metal Archives | `ma` |

An alias followed by a space searches only that engine: `gh niruwm`,
`aw pipewire`, `yt darkthrone`. A full engine name, such as `github niruwm`, is
not an alias and searches normally.

The AI providers (ChatGPT, Gemini, Grok and Microsoft Copilot) open their web
application with the phrase in the prompt. They may require a signed-in account
and may not send the prompt automatically; the launcher does not log in for you
or send anything on its own.

Adding an engine is a few lines in `~/.config/nirulauncher/config.toml`; no
rebuild and no new mode:

```toml
[[search.sources]]
id = "searx"
label = "SearXNG (home)"
url = "https://searx.example/search?q={query}"
alias = "sx"
```

`{query}` is replaced by the percent-encoded phrase. An id that matches a
built-in replaces it in place (so the familiar engines keep their order); a new
id is appended after them. `enabled = false` turns an engine off. Only `http`
and `https` addresses are opened, so a mistyped template cannot start a local
program.

Order, default and alias overrides live in one table:

```toml
[search]
default = "duckduckgo"
order = ["duckduckgo", "github", "archwiki", "wikipedia"]
aliases = { gh = "github", aw = "archwiki", yt = "youtube" }
```

The launcher never contacts the network: it hands the address to the browser
named by `browser` in the launcher config, else NIRUWM's `browser`, else
`$BROWSER`, else the desktop default through `xdg-open`.

### Applications, keybindings and settings

Application matching accepts several words in any order, reads `.desktop`
`Keywords`, `GenericName` and `Comment`, and adds conservative fuzzy matching
after exact, prefix and word matches. Favourites still come first.

**NIRUX Settings** groups the existing tools: Appearance and themes, Wallpaper,
Displays, Audio, Keybindings and Idle and lock timing. **Displays** opens NIRU
Display when installed, otherwise the built-in profile view. **Keybindings** is read
from NIRUWM's active table, so it includes your overrides and leaves out
explicit unbinds. **Idle and lock timing** shows the effective minutes and lets
you type a new value; it is written to the shared `[idle]` section without
touching other settings or comments.

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
