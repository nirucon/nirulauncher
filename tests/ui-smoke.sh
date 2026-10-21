#!/bin/sh
# Real X11 smoke test: discovery, search, argv launch, Escape and singleton.
set -eu
BIN=${1:?usage: ui-smoke.sh BIN}
BIN=$(CDPATH='' cd -- "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
T=$(mktemp -d)
XVFB_PID=""
LAUNCHER=""
cleanup() {
  [ -z "$LAUNCHER" ] || kill "$LAUNCHER" 2>/dev/null || :
  [ -z "$XVFB_PID" ] || kill "$XVFB_PID" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
mkdir -p "$T/data/applications" "$T/home"
cat > "$T/fakeapp" <<'APP'
#!/bin/sh
printf '%s\n' "$@" > "$ARGS_FILE"
APP
chmod +x "$T/fakeapp"
cat > "$T/data/applications/test.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Test App
Exec=$T/fakeapp --one "two words"
DESKTOP
Xvfb :98 -screen 0 1280x800x24 -nolisten tcp >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X98 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:98 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
  XDG_DATA_HOME="$T/data" ARGS_FILE="$T/args"
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL launcher did not map"; exit 1; }
  sleep 0.05
done
xdotool type --delay 20 'test'
xdotool key Return
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL launcher did not exit"; exit 1; }
  sleep 0.05
done
LAUNCHER=""
[ "$(tr '\n' '|' < "$T/args")" = "--one|two words|" ] || {
  echo "FAIL wrong argv: $(cat "$T/args")"; exit 1;
}
rm -f "$T/args"
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo "FAIL second launcher did not map"; exit 1; }
  sleep 0.05
done
if "$BIN" --apps >"$T/second.out" 2>"$T/second.err"; then
  echo "FAIL singleton allowed a second process"; exit 1
fi
grep -q 'already open' "$T/second.err" || { cat "$T/second.err"; echo "FAIL singleton message"; exit 1; }
xdotool key Escape
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo "FAIL Escape did not close"; exit 1; }
  sleep 0.05
done
LAUNCHER=""
# Large fonts must grow rows instead of clipping. With max_rows=3 the window
# must be taller than the old fixed 26 px row layout.
mkdir -p "$XDG_CONFIG_HOME/nirulauncher"
printf 'max_rows = 3\nfont_size = 26.0\nrow_height = 26\n' > "$XDG_CONFIG_HOME/nirulauncher/config.toml"
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL large-font launcher did not map"; exit 1; }
  sleep 0.05
done
WIN=$(xdotool search --name '^NIRULANCHER$' | head -1)
HEIGHT=$(xdotool getwindowgeometry --shell "$WIN" | sed -n 's/^HEIGHT=//p')
[ -n "$HEIGHT" ] && [ "$HEIGHT" -gt 180 ] || {
  echo "FAIL large font did not grow the launcher (height=$HEIGHT)"
  exit 1
}
xdotool key Escape
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo "FAIL large-font Escape did not close"; exit 1; }
  sleep 0.05
done
LAUNCHER=""
echo "PASS ui-smoke"
