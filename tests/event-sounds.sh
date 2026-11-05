#!/bin/sh
# Event and per-app sounds in the Sound view, driven through a fake `nirux`
# (a JSON fixture plus an argv log) under Xvfb.
# Usage: event-sounds.sh /path/to/nirulauncher
set -eu
BIN=${1:?usage: event-sounds.sh BIN}
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
fail() { echo "FAIL $*"; echo '--- nirux argv:'; cat "$T/nirux.log" 2>/dev/null || :; cat "$T/err" 2>/dev/null || :; exit 1; }

mkdir -p "$T/bin" "$T/home/.config/nirux/sounds" "$T/prefix/share/nirux/sounds"
for name in notification mail chat; do
  printf 'OggS\n' > "$T/prefix/share/nirux/sounds/$name.ogg"
done
printf 'OggS\n' > "$T/home/.config/nirux/sounds/ping.ogg"
cat > "$T/status.json" <<JSON
{"schema":1,"enabled":true,"volume":40,"config":"$T/home/.config/nirux/sounds.toml","events":{"startup":true,"notification":true,"update":false,"shutdown":true},"exclude":["nplay"],"apps":[{"app":"thunderbird","sound":"mail","state":"ok","file":"$T/prefix/share/nirux/sounds/mail.ogg"},{"app":"signal","sound":"off","state":"silent","file":""},{"app":"discord","sound":"nosuchsound","state":"missing","file":""}]}
JSON
cat > "$T/bin/nirux" <<'NIRUX'
#!/bin/sh
printf '%s\n' "$*" >> "$NIRUX_LOG"
case "$*" in
  'sounds status --json')
    if [ -n "${OLD_NIRUX:-}" ]; then
      # NIRUX before per-app sounds: text status, no JSON.
      printf 'Sounds: on · volume 35%%\n'
      exit 0
    fi
    cat "$STATUS_JSON" ;;
  'sounds app '*) printf '%s = %s\n' "$3" "$4" ;;
  *) printf 'ok\n' ;;
esac
NIRUX
cat > "$T/bin/nirunotify" <<'NOTIFY'
#!/bin/sh
[ "$1" = --history ] || exit 2
printf '1\tThunderbird\tMail\n2\tKitty\tDone\n3\tElement\tMessage\n4\tGoogle Chrome\tTab\n'
NOTIFY
cat > "$T/bin/niructl" <<'CTL'
#!/bin/sh
exit 1
CTL
chmod +x "$T/bin/nirux" "$T/bin/nirunotify" "$T/bin/niructl"

Xvfb :96 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X96 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:96 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
  XDG_DATA_HOME="$T/data" XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/cache" \
  NIRUX_PREFIX="$T/prefix" NIRUX_LOG="$T/nirux.log" STATUS_JSON="$T/status.json" \
  NIRU_UI_SCALE=1 PATH="$T/bin:$PATH"

open_sound() {
  : > "$NIRUX_LOG"
  "$BIN" --sound >"$T/out" 2>"$T/err" &
  LAUNCHER=$!
  i=0
  while ! grep -qxF 'sounds status --json' "$NIRUX_LOG"; do
    i=$((i + 1)); [ "$i" -gt 100 ] && fail 'the Sound view did not read nirux sounds status --json'
    sleep 0.05
  done
  sleep 0.3
}
clear_query() { xdotool key --delay 5 --repeat 40 BackSpace; }
# Type a filter, press Enter and wait for one argv line.
act() {
  filter=$1 want=$2
  clear_query
  xdotool type --delay 15 "$filter"
  sleep 0.2
  xdotool key Return
  i=0
  while ! grep -qxF -- "$want" "$NIRUX_LOG"; do
    i=$((i + 1)); [ "$i" -gt 60 ] && fail "'$filter' did not run: nirux $want"
    sleep 0.05
  done
  sleep 0.2
  echo "PASS $filter -> nirux $want"
}
pick() {
  filter=$1
  clear_query
  xdotool type --delay 15 "$filter"
  sleep 0.2
  xdotool key Return
  sleep 0.4
}

open_sound
act 'all event sounds' 'sounds off'
act 'event update off' 'sounds toggle update'
act 'event startup test' 'sounds test startup'
act 'thunderbird test' 'sounds test notification --app thunderbird'
act 'discord remove' 'sounds app discord remove'
pick 'signal off'
act 'chat' 'sounds app signal chat'
pick 'signal off'
act 'default' 'sounds app signal default'
pick 'thunderbird mail'
act 'ping' 'sounds app thunderbird ping'
# Add app: a suggestion from the notification history.
pick 'add an app'
pick 'element'
act 'mail' 'sounds app element mail'
# Add app: a typed name.
pick 'add an app'
pick 'my.app_2'
act 'off' 'sounds app my.app_2 off'
# An invalid typed name never reaches nirux.
pick 'add an app'
pick 'bad name!'
sleep 0.3
xdotool key Escape
sleep 0.3
if grep -q '^sounds app bad' "$NIRUX_LOG"; then fail 'an invalid app name reached nirux'; fi
kill -0 "$LAUNCHER" 2>/dev/null || fail 'Escape in a choice list closed the launcher'
xdotool key Escape
wait "$LAUNCHER" 2>/dev/null || :
LAUNCHER=""
# Every change ran with argv only, then the state was read again.
reads=$(grep -c '^sounds status --json$' "$NIRUX_LOG")
changes=$(grep -cv '^sounds status --json$' "$NIRUX_LOG")
[ "$reads" = $((changes + 1)) ] || fail "changes did not re-read the status ($reads reads, $changes changes)"
echo 'PASS invalid app names are rejected in the launcher'

# An older NIRUX without --json: the section is unavailable and no row acts.
export OLD_NIRUX=1
open_sound
clear_query
xdotool type --delay 15 'all event sounds'
xdotool key Return
sleep 0.5
[ "$(grep -cv '^sounds status --json$' "$NIRUX_LOG")" = 0 ] || fail 'an older NIRUX got a change request'
kill -0 "$LAUNCHER" 2>/dev/null || fail 'the Sound view closed with an older NIRUX'
xdotool key Escape
wait "$LAUNCHER" 2>/dev/null || :
LAUNCHER=""
echo 'PASS older NIRUX: event sounds unavailable, nothing run'
unset OLD_NIRUX

# No nirux at all.
mv "$T/bin/nirux" "$T/nirux.off"
: > "$NIRUX_LOG"
PATH="$T/bin:/usr/bin:/bin" "$BIN" --sound >"$T/out" 2>"$T/err" &
LAUNCHER=$!
sleep 0.8
kill -0 "$LAUNCHER" 2>/dev/null || fail 'the Sound view closed without nirux'
xdotool key Escape
wait "$LAUNCHER" 2>/dev/null || :
LAUNCHER=""
echo 'PASS missing nirux: event sounds unavailable'
