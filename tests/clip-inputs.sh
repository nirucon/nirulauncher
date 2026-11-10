#!/bin/sh
# Microphone inputs of a multichannel interface in the Capture view, under Xvfb.
#
# 1. A fake niruclip reports an 18-input interface as the default source.
#    The Capture view shows "Mic inputs"; Enter opens the picker,
#    choosing "Inputs 3+4" remembers it and goes back to Capture.
# 2. Full screen and a profile then pass --mic-channels 3,4.
# 3. An older niruclip without the channel fields: no row, no flag.
#
# SHOTS=DIR saves screenshots of the views there.
# Usage: clip-inputs.sh /path/to/nirulauncher
set -eu
BIN=${1:?usage: clip-inputs.sh BIN}
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

UMC=alsa_input.usb-BEHRINGER_UMC1820.multichannel-input
mkdir -p "$T/bin" "$T/home" "$T/state"
cat > "$T/bin/niruclip" <<'FAKE'
#!/bin/sh
case "$*" in
  "--list-mics --json")
    sleep 1
    if [ -n "${OLD_NIRUCLIP:-}" ]; then
      echo '[{"name":"alsa_input.usb-BEHRINGER_UMC1820.multichannel-input","description":"UMC1820 Multichannel","default":true}]'
    else
      printf '[{"name":"alsa_input.usb-BEHRINGER_UMC1820.multichannel-input","description":"UMC1820 Multichannel","default":true,"channels":18,"channel_names":['
      i=0; while [ "$i" -lt 18 ]; do [ "$i" = 0 ] || printf ','; printf '"AUX%s"' "$i"; i=$((i + 1)); done
      echo '],"multichannel":true}]'
    fi ;;
  "--list-profiles") printf 'band\tBand practice\tfull\tmic\toff\n' ;;
  "--status --json") echo '{"state":"idle"}' ;;
  *) printf '%s\n' "niruclip $*" >> "$LOG" ;;
esac
FAKE
chmod +x "$T/bin/niruclip"

Xvfb :95 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X95 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:95 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
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
logged() {
  i=0
  while ! grep -qxF -- "$1" "$LOG"; do
    i=$((i + 1)); [ "$i" -gt 60 ] && fail "$2: log is '$(cat "$LOG")'"
    sleep 0.05
  done
}
shot() { [ -z "${SHOTS:-}" ] || import -window root "$SHOTS/$1.png"; }

open --clip
sleep 1.2
shot capture
xdotool type --delay 20 'mic inputs'
sleep 0.2
xdotool key Return
sleep 1.4
shot picker
[ -n "$(window)" ] || fail 'the picker closed the launcher'
xdotool type --delay 20 '3+4'
sleep 0.2
xdotool key Return
i=0
while ! grep -qxF "$UMC	3,4" "$T/state/nirulauncher/clip-inputs.tsv" 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 60 ] && fail 'choice not remembered'
  sleep 0.05
done
sleep 1.4
shot chosen
[ -n "$(window)" ] || fail 'choosing closed the launcher'
echo 'PASS the picker remembers Inputs 3+4 for the interface and goes back'
pkill -f "^$BIN" 2>/dev/null || :
closed picker

# Enter right away waits for the microphones, then records with the inputs.
open --clip
xdotool type --delay 10 'full screen'
xdotool key Return
closed 'full screen'
logged "niruclip --full --audio mic --countdown 3 --mic-channels 3,4" 'full screen'
echo 'PASS Full screen passes --mic-channels 3,4'

open --clip
xdotool type --delay 20 'band practice'
sleep 1.2
xdotool key Return
closed profile
logged "niruclip --profile band --countdown 3 --mic-channels 3,4" profile
echo 'PASS a profile on the interface passes --mic-channels 3,4'

: > "$LOG"
export OLD_NIRUCLIP=1
open --clip
sleep 1.2
shot old
xdotool type --delay 10 'full screen'
xdotool key Return
closed 'old niruclip'
logged "niruclip --full --audio mic --countdown 3" 'old niruclip'
echo 'PASS an older niruclip: no inputs row and no --mic-channels'
