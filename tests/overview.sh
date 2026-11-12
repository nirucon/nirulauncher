#!/bin/sh
# Workspace overview, `--apps --query --launch-best` and `--note --stdin`
# against a real X server: a hand-made NIRUWM state with two monitors,
# generated thumbnails and a stub niructl that records its argv.
# Usage: overview.sh BIN [SCREENSHOT_DIR]
set -eu
BIN=${1:?usage: overview.sh BIN [SCREENSHOT_DIR]}
BIN=$(CDPATH='' cd -- "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
SHOTS=${2:-}
T=$(mktemp -d)
XVFB_PID=""
LAUNCHER=""
cleanup() {
  [ -z "$LAUNCHER" ] || kill "$LAUNCHER" 2>/dev/null || :
  [ -z "$XVFB_PID" ] || kill "$XVFB_PID" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL $*"; [ ! -f "$T/err" ] || cat "$T/err"; exit 1; }

Xvfb :93 -screen 0 3840x1080x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X93 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:93
export HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_DATA_HOME="$T/data"
export XDG_STATE_HOME="$T/state" XDG_RUNTIME_DIR="$T/run"
mkdir -p "$XDG_CONFIG_HOME/niruwm" "$XDG_DATA_HOME/applications" "$T/bin"
mkdir -p "$XDG_RUNTIME_DIR/niruwm/overview"
chmod 0700 "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/niruwm/overview"
D="$XDG_RUNTIME_DIR/niruwm/overview"

cat > "$T/bin/niructl" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" >> "$NIRUCTL_LOG"
STUB
chmod +x "$T/bin/niructl"
export PATH="$T/bin:$PATH" NIRUCTL_LOG="$T/niructl.log"

# Thumbnails: shapes on gradients, no fonts needed.
thumb() {
  convert -size "$2" "gradient:$3" -fill "$4" -draw "rectangle 20,20 180,120" \
    -fill '#f0f0f0' -draw "rectangle 200,40 360,190" "ppm:$D/$1"
}
thumb mon0-tag1.ppm 384x216 '#1d3557-#457b9d' '#e63946'
thumb mon0-tag2.ppm 384x216 '#2a9d8f-#264653' '#e9c46a'
thumb mon0-tag7.ppm 384x216 '#6d597a-#355070' '#eaac8b'
thumb mon1-tag1.ppm 384x162 '#3a5a40-#a3b18a' '#dad7cd'
printf 'P6\n384 216\n255\n\001\002' > "$D/mon0-tag5.ppm"   # cut short

cat > "$D/state.json" <<'JSON'
{"version":1,
 "monitors":[
  {"index":0,"name":"DP-4","x":0,"y":0,"width":1920,"height":1080,"focused":true,"current_tag":2,
   "tags":[
    {"tag":1,"name":"","thumbnail":"mon0-tag1.ppm","captured":1,
     "windows":[{"id":"0x400003","class":"firefox","title":"Mozilla Firefox: a very long page title that cannot fit on one card","focused":true}]},
    {"tag":2,"name":"","thumbnail":"mon0-tag2.ppm","captured":1,
     "windows":[{"id":"0x400007","class":"kitty","title":"~/projects","focused":true},
                {"id":"0x400009","class":"nirunote","title":"inbox.md","focused":false}]},
    {"tag":3,"name":"","thumbnail":"","captured":0,
     "windows":[{"id":"0x500001","class":"Gimp","title":"GIMP","focused":false}]},
    {"tag":5,"name":"","thumbnail":"mon0-tag5.ppm","captured":1,
     "windows":[{"id":"0x500002","class":"mpv","title":"film.mkv","focused":true}]},
    {"tag":7,"name":"mail","thumbnail":"mon0-tag7.ppm","captured":1,
     "windows":[{"id":"1","class":"a","title":"Inkorg","focused":false},{"id":"2","class":"b","title":"Two","focused":false},
                {"id":"3","class":"c","title":"Three","focused":false},{"id":"4","class":"d","title":"Four","focused":false}]}]},
  {"index":1,"name":"HDMI-1","x":1920,"y":0,"width":2560,"height":1080,"focused":false,"current_tag":6,
   "tags":[
    {"tag":1,"name":"","thumbnail":"mon1-tag1.ppm","captured":1,
     "windows":[{"id":"0x600001","class":"signal","title":"Signal","focused":true}]},
    {"tag":4,"name":"","thumbnail":"mon1-tag4.ppm","captured":1,"windows":[]}]}]}
JSON

open_view() {
  : > "$T/err"
  "$BIN" "$@" >"$T/out" 2>"$T/err" &
  LAUNCHER=$!
  i=0
  WIN=""
  while [ "$i" -lt 100 ]; do
    WIN=$(xdotool search --name '^NIRULANCHER$' 2>/dev/null | head -1 || true)
    [ -n "$WIN" ] && break
    i=$((i + 1)); sleep 0.05
  done
  [ -n "$WIN" ] || fail "$*: no window"
  sleep 0.3
}

closed() {
  i=0
  while kill -0 "$LAUNCHER" 2>/dev/null; do
    i=$((i + 1)); [ "$i" -gt 60 ] && fail "$1: still open"
    sleep 0.05
  done
  wait "$LAUNCHER" 2>/dev/null || :
  LAUNCHER=""
}

last_call() { tail -1 "$NIRUCTL_LOG" 2>/dev/null || :; }

shot() {
  [ -z "$SHOTS" ] || import -window "$WIN" "$SHOTS/$1.png"
}

for theme in niru-noir niru-carl-larsson; do
  printf 'theme = "%s"\n' "$theme" > "$XDG_CONFIG_HOME/niruwm/config.toml"
  open_view --overview
  # The frame must have content: more than a handful of colours.
  xwd -id "$WIN" -silent > "$T/win.xwd"
  colours=$(convert "$T/win.xwd" -format %k info:)
  [ "$colours" -gt 50 ] || fail "$theme: overview has only $colours colours"
  # The thumbnail's red square is on screen.
  convert "$T/win.xwd" -format %c histogram:info:- | grep -qi '#e63946' \
    || fail "$theme: the tag 1 thumbnail is not drawn"
  shot "overview-$theme"
  xdotool key Escape
  closed "$theme Escape"
  echo "PASS overview draws in $theme ($colours colours)"
done
[ ! -s "$NIRUCTL_LOG" ] || fail "Escape ran niructl: $(cat "$NIRUCTL_LOG")"

# Enter on the start card: the focused monitor's current tag and its window.
open_view --overview
xdotool key Return
closed "Enter"
[ "$(last_call)" = "do goto 0 2 0x400007" ] || fail "Enter ran: $(last_call)"
echo "PASS Enter picks the current tag"

# Right then Enter: tag 3 has no focused window.
open_view --overview
xdotool key Right
sleep 0.2
shot overview-moved
xdotool key Return
closed "Right Enter"
[ "$(last_call)" = "do goto 0 3" ] || fail "Right Enter ran: $(last_call)"
echo "PASS arrows move"

# Down from the first monitor's top row reaches the second monitor.
open_view --overview
xdotool key Home Down Down Return
closed "Down"
case "$(last_call)" in "do goto 1 "*) ;; *) fail "Down Down ran: $(last_call)" ;; esac
echo "PASS Down crosses to the second monitor ($(last_call))"

# A digit goes straight to that tag on the focused monitor.
open_view --overview
xdotool key 5
closed "digit"
[ "$(last_call)" = "do goto 0 5 0x500002" ] || fail "5 ran: $(last_call)"
open_view --overview
xdotool key 9
closed "digit 9"
[ "$(last_call)" = "do goto 0 9" ] || fail "9 ran: $(last_call)"
echo "PASS digits pick tags"

# A click on the first card picks tag 1; a click outside closes.
open_view --overview
geometry=$(xdotool getwindowgeometry "$WIN")
X=$(printf '%s\n' "$geometry" | sed -n 's/.*Position: \([0-9]*\),\([0-9]*\).*/\1/p')
Y=$(printf '%s\n' "$geometry" | sed -n 's/.*Position: \([0-9]*\),\([0-9]*\).*/\2/p')
xdotool mousemove $((X + 60)) $((Y + 140)) click 1
closed "click card"
[ "$(last_call)" = "do goto 0 1 0x400003" ] || fail "click ran: $(last_call)"
calls=$(wc -l < "$NIRUCTL_LOG")
open_view --overview
xdotool mousemove 3000 900 click 1
closed "click outside"
[ "$(wc -l < "$NIRUCTL_LOG")" = "$calls" ] || fail "click outside ran: $(last_call)"
echo "PASS mouse picks and closes"

# No state at all: a short message that any key closes.
rm "$D/state.json"
open_view --overview
shot overview-nostate
grep -q 'no workspace state' "$T/err" || fail "missing state: $(cat "$T/err")"
xdotool key space
closed "message"
printf '{"version":9,"monitors":[]}' > "$D/state.json"
open_view --overview
grep -q 'version 9' "$T/err" || fail "new version: $(cat "$T/err")"
xdotool key Escape
closed "version message"
[ "$(wc -l < "$NIRUCTL_LOG")" = "$calls" ] || fail "the message ran niructl"
echo "PASS missing and unknown state show a message"

# --apps --query --launch-best
cat > "$T/fakeapp" <<'APP'
#!/bin/sh
printf '%s\n' "$0 $*" >> "$APP_LOG"
APP
chmod +x "$T/fakeapp"
export APP_LOG="$T/apps.log"
desk() {
  printf '[Desktop Entry]\nType=Application\nName=%s\nKeywords=%s\nExec=%s %s\n' \
    "$2" "$3" "$T/fakeapp" "$1" > "$XDG_DATA_HOME/applications/$1.desktop"
}
desk nirunote NIRUNOTE 'markdown;notes;'
desk kitty kitty 'terminal;shell;'
desk alacritty Alacritty 'terminal;shell;'
desk calc 'Miniräknare' 'kalkylator;'
out=$("$BIN" --apps --query 'niru note' --launch-best)
[ "$out" = "$(printf 'launched\tNIRUNOTE')" ] || fail "launch-best printed: $out"
sleep 0.2
grep -q "fakeapp nirunote" "$APP_LOG" || fail "nirunote was not started"
out=$("$BIN" --apps --query miniraknare --launch-best)
[ "$out" = "$(printf 'launched\tMiniräknare')" ] || fail "launch-best printed: $out"
grep -q nirunote "$XDG_STATE_HOME/nirulauncher/usage.tsv" || fail "the launch was not recorded"
echo "PASS --launch-best starts a confident match"

: > "$T/err"
"$BIN" --apps --query terminal --launch-best >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while [ "$(cat "$T/out")" != "shown" ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && fail "terminal: printed $(cat "$T/out")"
  sleep 0.05
done
WIN=$(xdotool search --name '^NIRULANCHER$' | head -1)
[ -n "$WIN" ] || fail "terminal: no Apps window"
sleep 0.2
shot launch-best-shown
xdotool key Escape
closed "Apps Escape"
grep -q "fakeapp kitty\|fakeapp alacritty" "$APP_LOG" && fail "an ambiguous terminal was started"
echo "PASS --launch-best shows the Apps view when unsure"

# --note --stdin
note=$(printf '  ring Anna\nom mötet\n' | "$BIN" --note --stdin)
[ "$note" = "$HOME/Notes/inbox.md" ] || fail "note printed: $note"
grep -q ' ring Anna om mötet$' "$note" || fail "note text: $(cat "$note")"
if printf ' \n' | "$BIN" --note --stdin 2>/dev/null; then fail "an empty note exited 0"; fi
echo "PASS --note --stdin"
echo "PASS overview"
