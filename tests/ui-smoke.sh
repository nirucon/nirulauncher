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
# The large-font check below needs three rows. Do not depend on whichever
# system .desktop files happen to be installed in the test distribution.
for fixture in one two; do
  cat > "$T/data/applications/layout-$fixture.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Layout fixture $fixture
Exec=$T/fakeapp $fixture
DESKTOP
done
Xvfb :98 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
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
# Paging: End selects the last row and Home the first; Ctrl+Backspace
# deletes the last word of the query.
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL paging launcher did not map"; exit 1; }
  sleep 0.05
done
sleep 0.3
xdotool type --delay 20 'fixture zzz'
xdotool key ctrl+BackSpace
xdotool key End Home End
xdotool key Return
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL paging launch did not exit"; exit 1; }
  sleep 0.05
done
LAUNCHER=""
# "fixture" matches the two layout fixtures; End picks the second one.
[ "$(cat "$T/args" 2>/dev/null)" = two ] || { echo "FAIL End did not select the last fixture: $(cat "$T/args" 2>/dev/null)"; exit 1; }
rm -f "$T/args"
# Dead keys compose in the search field: dead_acute then e is é, which finds
# "Café Écran" ahead of every row that only matches a folded e. xdotool maps
# the dead key onto a spare keycode first, so this also proves the cached
# keyboard map is refreshed on MappingNotify.
cat > "$T/data/applications/cafe.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Café Écran
Exec=$T/fakeapp --cafe
DESKTOP
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL compose launcher did not map"; exit 1; }
  sleep 0.05
done
xdotool key --delay 40 dead_acute e
xdotool type --delay 20 'cran'
xdotool key Return
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo "FAIL compose launch did not exit"; exit 1; }
  sleep 0.05
done
LAUNCHER=""
[ "$(cat "$T/args")" = "--cafe" ] || { echo "FAIL dead key did not compose é: $(cat "$T/args")"; exit 1; }
rm -f "$T/args" "$T/data/applications/cafe.desktop"
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
# A dedicated microphone view reads inputs without changing anything; Enter
# selects the exact source ID, including when the name contains spaces.
mkdir -p "$T/bin"
cat > "$T/bin/niructl" <<'AUDIO'
#!/bin/sh
case "$*" in
  'audio sources') printf '*53\tBuilt-in microphone\n67\tUSB podcast microphone\n' ;;
  *) printf '%s\n' "$*" >> "$ARGS_FILE" ;;
esac
AUDIO
chmod +x "$T/bin/niructl"
export PATH="$T/bin:$PATH"
rm -f "$ARGS_FILE"
"$BIN" --microphone >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL microphone view'; exit 1; }
  sleep 0.05
done
[ ! -s "$ARGS_FILE" ] || { echo 'FAIL opening microphone view changed audio'; exit 1; }
xdotool type --delay 20 'podcast'
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL microphone selection'; exit 1; }
  sleep 0.05
done
grep -qxF 'audio source 67' "$ARGS_FILE"
wait "$LAUNCHER"
LAUNCHER=""

# Calculator: an arithmetic query shows "= 8" first; Enter copies 8 through
# xclip's stdin (never argv) and closes the launcher.
cat > "$T/bin/xclip" <<'CLIP'
#!/bin/sh
printf '%s\n' "$*" > "$ARGS_FILE.argv"
cat > "$ARGS_FILE"
CLIP
chmod +x "$T/bin/xclip"
rm -f "$ARGS_FILE" "$ARGS_FILE.argv"
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL calculator view'; exit 1; }
  sleep 0.05
done
xdotool type --delay 20 '2+2*3'
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL calculator did not copy'; exit 1; }
  sleep 0.05
done
[ "$(cat "$ARGS_FILE")" = 8 ] || { echo "FAIL calculator copied $(cat "$ARGS_FILE")"; exit 1; }
grep -qxF -- '-selection clipboard -in' "$ARGS_FILE.argv" || { echo 'FAIL xclip argv'; exit 1; }
wait "$LAUNCHER"
LAUNCHER=""
rm -f "$T/bin/xclip" "$ARGS_FILE" "$ARGS_FILE.argv"

# Web search: a phrase opens the configured browser with the engine's own
# address, and Tab moves to the next engine before Enter is pressed.
cat > "$T/bin/fakebrowser" <<'BROWSER'
#!/bin/sh
printf '%s\n' "$@" >> "$ARGS_FILE"
BROWSER
chmod +x "$T/bin/fakebrowser"
mkdir -p "$XDG_CONFIG_HOME/nirulauncher"
printf 'browser = ["%s"]\n' "$T/bin/fakebrowser" > "$XDG_CONFIG_HOME/nirulauncher/config.toml"
rm -f "$ARGS_FILE"
"$BIN" --search 'black metal' >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL search view'; exit 1; }
  sleep 0.05
done
# The phrase arrives on the command line; one Tab picks the second engine.
xdotool key Tab
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL search did not open a browser'; exit 1; }
  sleep 0.05
done
wait "$LAUNCHER"
LAUNCHER=""
grep -qxF 'https://search.brave.com/search?q=black+metal' "$ARGS_FILE" || {
  echo "FAIL wrong search address: $(cat "$ARGS_FILE")"; exit 1
}

# Typing the phrase instead of passing it gives the same result.
rm -f "$ARGS_FILE"
"$BIN" --search >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL empty search view'; exit 1; }
  sleep 0.05
done
xdotool type --delay 40 'metal archives'
sleep 1
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL typed search did not open a browser'; exit 1; }
  sleep 0.05
done
wait "$LAUNCHER"
LAUNCHER=""
grep -qxF 'https://duckduckgo.com/?q=metal+archives' "$ARGS_FILE" || {
  echo "FAIL wrong typed address: $(cat "$ARGS_FILE")"; exit 1
}
# No matches: the list says so, and Tab moves the typed text to the web
# search in the same window.
rm -f "$ARGS_FILE"
"$BIN" --apps >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL no-match view'; exit 1; }
  sleep 0.05
done
xdotool type --delay 20 'zzqqxx'
sleep 0.3
xdotool key Tab
sleep 0.2
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL Tab did not switch to web search'; exit 1; }
  sleep 0.05
done
wait "$LAUNCHER"
LAUNCHER=""
grep -qxF 'https://duckduckgo.com/?q=zzqqxx' "$ARGS_FILE" || {
  echo "FAIL wrong no-match search: $(cat "$ARGS_FILE")"; exit 1
}
# Naming a layout profile happens in the launcher itself: the row opens a
# prompt, and Enter saves the profile with the typed name.
cat > "$T/bin/niructl" <<'SAVE'
#!/bin/sh
case "$*" in
  'display list') exit 0 ;;
  'display save'*) printf '%s
' "$*" >> "$ARGS_FILE" ;;
esac
SAVE
chmod +x "$T/bin/niructl"
rm -f "$ARGS_FILE"
"$BIN" --displays >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! xdotool search --name '^NIRULANCHER$' >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/err"; echo 'FAIL displays view'; exit 1; }
  sleep 0.05
done
xdotool type --delay 20 'save this layout'
xdotool key Return
sleep 0.3
xdotool type --delay 20 'Desk 2'
xdotool key Return
i=0
while [ ! -s "$ARGS_FILE" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL layout profile was not saved'; exit 1; }
  sleep 0.05
done
grep -qxF 'display save Desk 2' "$ARGS_FILE" || {
  echo "FAIL wrong save call: $(cat "$ARGS_FILE")"; exit 1
}
xdotool key Escape
i=0
while kill -0 "$LAUNCHER" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { echo 'FAIL Escape after saving'; exit 1; }
  sleep 0.05
done
LAUNCHER=""
echo "PASS ui-smoke"
