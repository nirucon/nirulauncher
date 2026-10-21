# NIRULAUNCHER

Native application and desktop-action launcher for the NIRU X11 desktop.
**A personal project for the author's own machines; others are welcome to try it.**

- **Super+P:** focused application search and launch.
- **Super+Shift+P:** applications, open windows and grouped desktop actions.
- **`--windows`:** switch directly to an open window.
- Local favourites and recency order the results; no daemon or network.
- Keyboard-first, one process per invocation, no daemon, indexing service or network.
- Reads suite colours from `~/.config/niruwm/config.toml` and user theme files.
- Independent settings in `~/.config/nirulauncher/config.toml`.

## Install

Use [NIRUX](https://github.com/nirucon/nirux#install) for the complete desktop on
**Void glibc, Debian and Arch Linux**, or install independently:

```sh
git clone https://github.com/nirucon/nirulauncher
cd nirulauncher
./install.sh              # release binary
./install.sh --build      # from this checkout
```

Requires an X11 session and a monospaced font (fontconfig or `font_path`).
Current release assets target x86_64/glibc; the release workflow builds static
musl binaries.

## Use

```sh
nirulauncher --apps
nirulauncher --actions
nirulauncher --windows
nirulauncher --microphone
nirulauncher --check-config
```

`--actions` groups System, Display, Power, Appearance, Workspaces and Windows
results; applications follow. Rows grow with the configured font, and the
selected row uses a 3 px theme-accent focus bar. Audio outputs and microphones
are listed as actions through `niructl audio`, alongside output/microphone mute.
`--microphone` opens only input selection, mute and input-gain controls, usable
with arrows/search and Enter. Opening it changes nothing. Gain changes preserve
mute, and missing inputs are explained. These controls also appear in `--actions`.
The Diagnostics group explains unavailable services from `niructl explain`, with
a direct action to the full `nirux doctor` report. Work modes start timed
focus/presentation DND and keep-awake; launcher recipes run configured app
groups into their workspaces. Lock, screensaver, reload, DND, idle inhibition,
brightness, night light, suspend, themes, workspaces and confirmed
logout/reboot/shutdown use the documented component CLIs. Applications with
`Terminal=true` use the configured `terminal` argv. Desktop `Exec` lines are
parsed into argv, never passed through a shell. Favourites are configured in
`~/.config/nirulauncher/config.toml`; launch recency is stored locally under
`~/.local/state/nirulauncher/`.

Limitations in 0.1: no input-method framework (dead-key compose sequences are
not composed), English `Name` keys, and Latin-1-only keysym text. Shift, Caps
Lock and AltGr levels are handled, including Swedish letters. The implementation
contract is in [DESIGN.md](DESIGN.md); the authoritative roadmap is
[NIRUX roadmap](https://github.com/nirucon/nirux/blob/main/ROADMAP.md).

MIT. Copyright 2026 Nicklas Rudolfsson.
