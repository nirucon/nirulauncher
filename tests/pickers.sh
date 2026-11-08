#!/bin/sh
# Emoji picker, Colors view and keybinding cheat sheet under Xvfb.
#
# 1. --emoji: search, Shift+Enter copies the emoji; Tab changes the skin
#    tone and it is remembered; the picked emoji is first next time.
# 2. Enter with no window to paste into leaves the emoji on the clipboard.
# 3. --colors: Enter copies the hex of the first picked color.
# 4. --keys: rows come from `niructl keys --json`; Enter runs
#    `niructl keys run KEYS`; a registered app key runs its own argv.
#
# Usage: pickers.sh /path/to/nirulauncher
set -eu
BIN=${1:?usage: pickers.sh BIN}
BIN=$(CDPATH='' cd -- "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
T=$(mktemp -d)
XVFB_PID=""
cleanup() {
  pkill -f "^$BIN" 2>/dev/null || :
  [ -z "$XVFB_PID" ] || kill "$XVFB_PID" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL $*"; [ ! -s "$T/err" ] || cat "$T/err"; exit 1; }

mkdir -p "$T/bin" "$T/home" "$T/state/nirushot" "$T/state/nirulauncher"
cat > "$T/bin/niructl" <<'FAKE'
#!/bin/sh
printf '%s\n' "niructl $*" >> "$LOG"
if [ "$1 $2" = "keys --json" ]; then
  cat <<'JSON'
{"version":1,"entries":[
 {"keys":"super+t","label":"Super+T","mods":64,"keysym":116,"action":"layout tile","run":null,
  "description":"Layout: Tile","group":"Layouts","source":"niruwm","app":null,"conflicts":[]},
 {"keys":"ctrl+alt+space","label":"Ctrl+Alt+Space","mods":12,"keysym":32,"action":null,
  "run":["fakeapp","toggle"],"description":"Dictate","group":"Dictation","source":"app",
  "app":"NIRUDICTATE","conflicts":[]}],"problems":[]}
JSON
fi
FAKE
printf '#!/bin/sh\nprintf "%%s\\n" "fakeapp $*" >> "$LOG"\n' > "$T/bin/fakeapp"
chmod +x "$T/bin/niructl" "$T/bin/fakeapp"
printf '[{"hex":"#ff8800","time":1},{"hex":"#112233","time":0}]' > "$T/state/nirushot/colors.json"

Xvfb :96 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X96 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:96 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
  XDG_DATA_HOME="$T/data" XDG_STATE_HOME="$T/state" LOG="$T/log" \
  PATH="$T/bin:$PATH"
: > "$LOG"

window() { xdotool search --name '^NIRULANCHER$' 2>/dev/null | head -1; }
open() {
  "$BIN" "$@" >"$T/out" 2>"$T/err" &
  i=0
  while [ -z "$(window)" ]; do
    i=$((i + 1)); [ "$i" -gt 100 ] && fail "$*: no window"
    sleep 0.05
  done
  sleep 0.3
}
closed() {
  i=0
  while [ -n "$(window)" ]; do
    i=$((i + 1)); [ "$i" -gt 100 ] && fail "$1: window stayed open"
    sleep 0.05
  done
}
clip() {
  i=0
  while [ "$(xclip -selection clipboard -o 2>/dev/null)" != "$1" ]; do
    i=$((i + 1)); [ "$i" -gt 60 ] && fail "$2: clipboard is '$(xclip -selection clipboard -o 2>/dev/null)', want '$1'"
    sleep 0.05
  done
}

open --emoji
xdotool type --delay 20 'waving hand'
sleep 0.2
xdotool key Tab Tab
xdotool key shift+Return
closed emoji
clip '👋🏼' 'Shift+Enter with tone 2'
echo 'PASS Shift+Enter copies the emoji with the chosen skin tone'
[ "$(cat "$T/state/nirulauncher/emoji-tone")" = 2 ] || fail 'tone not remembered'
grep -qx '👋' "$T/state/nirulauncher/emoji-recent.txt" || fail 'recent not recorded'
echo 'PASS the tone and the recent emoji are remembered'

open --emoji
xdotool key Return
closed 'emoji enter'
clip '👋🏼' 'Enter without a target window'
echo 'PASS the recent emoji is first; with no window to paste into it stays on the clipboard'
pkill -f "^$BIN" 2>/dev/null || :

open --emoji
xdotool type --delay 20 'copyright sign'
sleep 0.2
xdotool key shift+Return
closed symbol
clip '©' 'text symbol'
echo 'PASS text symbols are offered too'

open --colors
xdotool key Down Return
closed colors
clip '#ff8800' 'Colors view'
echo 'PASS Colors: Enter copies the hex of the newest pick'

open --keys
xdotool type --delay 20 'super+t'
sleep 0.2
xdotool key Return
closed keys
i=0
while ! grep -qx 'niructl keys run super+t' "$LOG"; do
  i=$((i + 1)); [ "$i" -gt 60 ] && fail "Enter did not run the binding: $(cat "$LOG")"
  sleep 0.05
done
echo 'PASS cheat sheet: a typed key finds its row and Enter runs it'

open --keys
xdotool type --delay 20 'dictate'
sleep 0.2
xdotool key Return
closed 'app key'
i=0
while ! grep -qx 'fakeapp toggle' "$LOG"; do
  i=$((i + 1)); [ "$i" -gt 60 ] && fail "app key did not run: $(cat "$LOG")"
  sleep 0.05
done
echo 'PASS cheat sheet: a registered app key runs its own command'
