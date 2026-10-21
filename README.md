# NIRULAUNCHER

Keyboard-first application, window and desktop-action launcher for the NIRU X11
desktop. One process per invocation: no daemon, indexing service or network.

Part of the [NIRUX suite](https://github.com/nirucon/nirux): [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot).

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

`--actions` groups System, Display, Power, Appearance, Workspaces and Windows.
It exposes audio sources and outputs, microphone mute/gain, Bluetooth device
actions, display-profile preview/keep/revert, power profiles, diagnostics, work
modes, recipes, lock, reload, DND, idle inhibition, brightness, night light,
themes, workspaces and confirmed session actions.

Desktop `Exec` lines are parsed into argv and never passed through a shell.
Applications with `Terminal=true` use the configured terminal argv. Favourites
are configured in `~/.config/nirulauncher/config.toml`; launch recency is stored
under `~/.local/state/nirulauncher/`. Colours and font come from
`~/.config/niruwm/config.toml`.

Current limits: no input-method framework, English `Name` keys, and Latin-1-only
keysym text. Shift, Caps Lock and AltGr are handled, including Swedish letters.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
sh tests/ui-smoke.sh
```

MIT. Copyright 2026 Nicklas Rudolfsson.
