#!/bin/sh
# Exact rendered focus/background colours for every built-in launcher theme.
# Usage: theme-colors.sh /path/to/nirulauncher
set -eu
BIN="${1:?path to nirulauncher binary}"
export DISPLAY=:92
T=$(mktemp -d)
XVFB=""
LAUNCHER=""
cleanup() {
  [ -z "$LAUNCHER" ] || kill "$LAUNCHER" 2>/dev/null || :
  [ -z "$XVFB" ] || kill "$XVFB" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
Xvfb :92 -screen 0 800x600x24 -nolisten tcp >"$T/xvfb.log" 2>&1 &
XVFB=$!
i=0
while [ ! -S /tmp/.X11-unix/X92 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_DATA_HOME="$T/data"
mkdir -p "$XDG_CONFIG_HOME/niruwm" "$XDG_DATA_HOME/applications"
printf '[Desktop Entry]\nType=Application\nName=Test\nExec=true\n' > "$XDG_DATA_HOME/applications/test.desktop"

check_theme() {
  theme=$1
  background=$2
  accent=$3
  printf 'theme = "%s"\n' "$theme" > "$XDG_CONFIG_HOME/niruwm/config.toml"
  "$BIN" --apps >"$T/out" 2>"$T/err" &
  LAUNCHER=$!
  i=0
  while [ "$i" -lt 100 ]; do
    WIN=$(xdotool search --name '^NIRULANCHER$' 2>/dev/null | head -1 || true)
    [ -n "$WIN" ] && break
    i=$((i + 1)); sleep 0.05
  done
  [ -n "${WIN:-}" ] || { cat "$T/err"; echo "FAIL $theme: no window"; exit 1; }
  sleep 0.2
  xwd -id "$WIN" -silent > "$T/win.xwd"
  colours=$(convert "$T/win.xwd" -format %c histogram:info:- 2>/dev/null)
  printf '%s\n' "$colours" | grep -qi "#$accent" || {
    echo "FAIL $theme: focus $accent not rendered"
    printf '%s\n' "$colours" | head -20
    exit 1
  }
  printf '%s\n' "$colours" | grep -qi "#$background" || {
    echo "FAIL $theme: background $background not rendered"
    printf '%s\n' "$colours" | head -20
    exit 1
  }
  echo "PASS $theme: background=$background focus=$accent"
  kill "$LAUNCHER" 2>/dev/null || :
  wait "$LAUNCHER" 2>/dev/null || :
  LAUNCHER=""
  sleep 0.2
}

check_theme niru-noir 0f0f10 5a5a60
check_theme niru-nord 2e3440 88c0d0
check_theme niru-satie 141414 b08d7d
check_theme niru-carl-larsson f3ece0 a63d2f
echo "PASS rendered launcher theme colours"
