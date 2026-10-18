# Implementation contract — M2

This document is a design, not a claim that the listed interfaces exist yet.
Read nirux/ROADMAP.md and the current component code before implementation.

## Initial deliverable

Rust, x11rb, native text rendering; start on demand, no resident indexing daemon.
One instance per X display. A new activation raises/focuses the existing window
rather than leaving duplicate input grabs. Normal WM-managed dialog window with
WM_CLASS, title, selection/focus and normal cleanup. Never take over the root or
override the screen lock. Escape and outside click dismiss without launching.

Proposed CLI: `nirulauncher [--apps|--actions]`, `--version`, `--check-config`.
Proposed config: XDG_CONFIG_HOME/nirulauncher/config.toml, with validated window
width/max-height/padding, visible row count, font, palette, search mode, terminal
argv and custom named action argv arrays. Write a complete config example only
when each option is actually implemented. Unreadable config must give a useful
error; `--check-config` works without X or launching any application.

## Application discovery

Scan XDG_DATA_HOME/applications and XDG_DATA_DIRS/applications with desktop-file
ID precedence (including Hidden masks). Honour Type, NoDisplay, Hidden,
OnlyShowIn/NotShowIn, TryExec and Terminal. Handle localized names explicitly;
UTF-8 names must display and search correctly. Set XDG_CURRENT_DESKTOP=NIRUWM
in the session when integrating. Index once per invocation; bounded file sizes
and entry count. A broken entry must not stop the rest of the list.

Parse Exec according to desktop-entry quoting and field-code rules; use argv
spawning, never `sh -c`. Test spaces, quotes, literal percent, %i/%c/%k and file/URL
field codes without supplied files. Reject unsupported malformed entries rather
than reinterpret them. Respect entry Path when valid. Honour terminal argv.

## Monitor and input

Determine active monitor using the focused window/WM monitor state, then pointer
and RandR fallbacks. Center inside that monitor's work area, not the union of all
screens. Clamp dimensions to small screens. Support negative monitor origins.
Do not invent a new conflicting definition of active monitor: inspect niruwm.

Keyboard: text entry, Backspace, Escape, arrows, Enter, scrolling with selection
kept visible. Handle keyboard mapping changes and non-Latin input deliberately;
do not claim full input-method support without implementing and testing it.

## Actions and integration

Action mode combines application results with explicit desktop actions. Start
with lock, screensaver, configuration reload, workspace selection and theme
selection through existing documented component CLIs. Logout/reboot/shutdown
require a second explicit confirmation. Custom actions are argv arrays in TOML.
No network or package probes in the launcher render/input loop.

WM integration must preserve user key overrides and retain a fallback launcher.
Only after real tests/release assets exist: add nirulauncher to nirux's pinned
manifest, all staging/version checks, status/doctor, legacy adoption, rollback,
uninstall, CI fixtures and real-release tests. Do not update only PROGRAMS.

## Required tests before 0.1

- Desktop-file fixtures for precedence, visibility, quoted Exec and injection.
- Search ordering and bounds; invalid/oversized configuration.
- Xvfb: placement, small screens, input, Escape, launch argv, singleton and cleanup.
- Synthetic two-monitor fixtures and a real RandR setup where supported.
- Idle use: no daemon or periodic work after closing; bounded RSS while open.
- Build and run on Arch, Debian and Void glibc; publish x86_64 binary + SHA256SUMS.
