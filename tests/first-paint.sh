#!/bin/sh
# First paint and provider memoization under Xvfb.
#
# 1. Every view maps within 150 ms while every provider sleeps 2 s, and
#    Escape closes it at once.
# 2. Enter pressed before provider rows arrive acts on them when they do.
# 3. With fast providers, one open that shows the actions catalog and then
#    the Health view asks each provider query at most once.
#
# Usage: first-paint.sh /path/to/nirulauncher
set -eu
BIN=${1:?usage: first-paint.sh BIN}
BIN=$(CDPATH='' cd -- "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
BUDGET_MS=${FIRST_PAINT_BUDGET_MS:-150}
T=$(mktemp -d)
XVFB_PID=""
LAUNCHER=""
cleanup() {
  [ -z "$LAUNCHER" ] || kill "$LAUNCHER" 2>/dev/null || :
  [ -z "$XVFB_PID" ] || kill "$XVFB_PID" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL $*"; [ ! -s "$T/err" ] || cat "$T/err"; exit 1; }

mkdir -p "$T/bin" "$T/home" "$T/data/applications" "$T/state"
printf '[Desktop Entry]\nType=Application\nName=Paint fixture\nExec=true\n' \
  > "$T/data/applications/paint.desktop"
# Each provider logs its argv and sleeps for $PROVIDER_SLEEP seconds.
for name in niructl nirux niruclip nirushare tailscale wpctl xrandr \
  bluetoothctl powerprofilesctl nmcli xinput nirunotify; do
  cat > "$T/bin/$name" <<PROVIDER
#!/bin/sh
printf '%s\n' "$name \$*" >> "\$PROVIDER_LOG"
sleep "\${PROVIDER_SLEEP:-0}"
PROVIDER
  chmod +x "$T/bin/$name"
done

Xvfb :97 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X97 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:97 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
  XDG_DATA_HOME="$T/data" XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/home/.cache" \
  PROVIDER_LOG="$T/providers" NIRU_UI_SCALE=1 PATH="$T/bin:$PATH"

now_ms() { echo $(($(date +%s%N) / 1000000)); }
window() { xdotool search --name '^NIRULANCHER$' 2>/dev/null | head -1; }

export PROVIDER_SLEEP=2
for mode in apps actions system-actions health displays keys themes timing \
  input microphone bluetooth power clip tailscale share settings sound wifi emoji colors files; do
  start=$(now_ms)
  "$BIN" "--$mode" >"$T/out" 2>"$T/err" &
  LAUNCHER=$!
  while [ -z "$(window)" ]; do
    [ $(($(now_ms) - start)) -lt 3000 ] || fail "$mode: no window"
    kill -0 "$LAUNCHER" 2>/dev/null || fail "$mode: exited"
  done
  mapped=$(($(now_ms) - start))
  [ "$mapped" -le "$BUDGET_MS" ] || fail "$mode: first paint after ${mapped} ms (budget ${BUDGET_MS} ms)"
  start=$(now_ms)
  xdotool key Escape
  # The window going away is the close; the process may linger as a zombie
  # until this shell waits for it.
  while [ -n "$(window)" ]; do
    [ $(($(now_ms) - start)) -lt 500 ] || fail "$mode: Escape did not close while providers were loading"
  done
  wait "$LAUNCHER" 2>/dev/null || :
  LAUNCHER=""
  echo "PASS $mode: mapped in ${mapped} ms with 2 s providers"
done

# Enter typed before the provider rows arrive acts on the best match once
# they are there (the display rows come from a provider that answers late).
: > "$PROVIDER_LOG"
"$BIN" --displays >"$T/out" 2>"$T/err" &
LAUNCHER=$!
while [ -z "$(window)" ]; do kill -0 "$LAUNCHER" 2>/dev/null || fail 'displays exited'; done
xdotool type --delay 5 'arrange'
xdotool key Return
i=0
while ! grep -qxF 'niructl display auto' "$PROVIDER_LOG"; do
  i=$((i + 1)); [ "$i" -gt 100 ] && fail 'early Enter was lost'
  sleep 0.05
done
wait "$LAUNCHER" 2>/dev/null || :
LAUNCHER=""
echo 'PASS early Enter waits for the provider rows'

# Memoization: the actions catalog, then Health, in one open.
export PROVIDER_SLEEP=0
: > "$PROVIDER_LOG"
"$BIN" --actions >"$T/out" 2>"$T/err" &
LAUNCHER=$!
i=0
while ! grep -q '^niructl recipe list' "$PROVIDER_LOG"; do
  i=$((i + 1)); [ "$i" -gt 200 ] && fail 'actions providers did not run'
  sleep 0.05
done
sleep 0.3
xdotool type --delay 10 'health overview'
xdotool key Return
i=0
while ! grep -q '^nirux status' "$PROVIDER_LOG"; do
  i=$((i + 1)); [ "$i" -gt 200 ] && fail 'Health view did not open'
  sleep 0.05
done
sleep 0.5
xdotool key Escape
wait "$LAUNCHER" 2>/dev/null || :
LAUNCHER=""
# Wrappers such as timeout(1) run the fake under the same argv; count the
# launcher's own queries, not the processes a fake starts.
duplicates=$(sort "$PROVIDER_LOG" | uniq -d)
[ -z "$duplicates" ] || fail "provider queried more than once in one open: $duplicates"
for shared in 'niructl monitors' 'niruclip --status --json'; do
  [ "$(grep -cxF "$shared" "$PROVIDER_LOG")" = 1 ] || fail "'$shared' was not shared between views"
done
echo "PASS memoization: $(wc -l < "$PROVIDER_LOG") provider queries, none repeated"
