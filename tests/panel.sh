#!/bin/sh
# NIRUPANEL against a real X server, with stand-in `tailscale`, `niructl`,
# terminal, `xdg-open` and `xclip` on PATH that record their argv:
#
# 1. The first frame comes at once while `tailscale status` takes 1 s.
# 2. The computers of the same user show; nothing the panel draws as a
#    target overlaps another or leaves the window.
# 3. Right-click and Ctrl+Shift+C copy a computer's address and name, a click
#    runs `ssh NAME` in the terminal, a middle-click opens sftp://NAME/.
# 4. Typing finds key bindings (`niructl keys --json`, read once); Enter
#    runs one through `niructl keys run`; one that closes a window asks first.
# 5. Without the modules, or without Tailscale, nothing is run for them.
#
# Usage: panel.sh BIN
set -eu
BIN=${1:?usage: panel.sh BIN}
BIN=$(CDPATH='' cd -- "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
BUDGET_MS=${PANEL_FIRST_FRAME_BUDGET_MS:-400}
T=$(mktemp -d)
XVFB_PID=""
PANEL=""
cleanup() {
  [ -z "$PANEL" ] || kill "$PANEL" 2>/dev/null || :
  [ -z "$XVFB_PID" ] || kill "$XVFB_PID" 2>/dev/null || :
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL $*"; [ ! -f "$T/err" ] || tail -c 3000 "$T/err"; exit 1; }

mkdir -p "$T/bin" "$T/bin-plain" "$T/home/.config/nirulauncher" "$T/data/applications" \
  "$T/sysdata" "$T/state" "$T/cache" "$T/run"
chmod 0700 "$T/run"
printf '[Desktop Entry]\nType=Application\nName=Paint fixture\nExec=true\n' \
  > "$T/data/applications/paint.desktop"
printf '[Desktop Entry]\nType=Application\nName=A rather long application name that wraps\nExec=true\n' \
  > "$T/data/applications/long.desktop"

# Each stand-in logs its argv (and xclip its input) and answers from a fixture.
log_stub() {
  cat > "$T/bin/$1" <<STUB
#!/bin/sh
PATH=/usr/bin:/bin
printf '%s\n' "\$*" >> "$T/$1.log"
$2
STUB
  chmod +x "$T/bin/$1"
}
log_stub tailscale 'sleep "${TS_SLEEP:-0}"; cat "'"$T"'/status.json"'
log_stub niructl 'case "$1 $2" in "keys --json") cat "'"$T"'/keys.json";; "keys run") ;; *) exit 1;; esac'
log_stub fake-term ''
log_stub xdg-open ''
log_stub xclip 'cat >> "'"$T"'/clipboard"; printf "\n" >> "'"$T"'/clipboard"'
cp "$T/bin/niructl" "$T/bin-plain/niructl"
cp "$T/bin/fake-term" "$T/bin-plain/fake-term"

cat > "$T/status.json" <<'JSON'
{"BackendState":"Running","MagicDNSSuffix":"ex.ts.net",
 "Self":{"HostName":"box","DNSName":"box.ex.ts.net.","OS":"linux","UserID":7,
   "TailscaleIPs":["100.64.0.1"],"Online":true,"CurAddr":"","Relay":"ams","LastSeen":"0001-01-01T00:00:00Z"},
 "Peer":{
  "k1":{"HostName":"L-5CD4","DNSName":"winwork.ex.ts.net.","OS":"windows","UserID":7,
    "TailscaleIPs":["100.64.0.5"],"Online":false,"Relay":"fra","LastSeen":"2026-11-25T10:00:00Z"},
  "k2":{"HostName":"othala","DNSName":"othala.ex.ts.net.","OS":"linux","UserID":7,
    "TailscaleIPs":["100.64.0.2"],"Online":true,"CurAddr":"1.2.3.4:41641","Relay":"ams"},
  "k3":{"HostName":"mac","DNSName":"mac.ex.ts.net.","OS":"macOS","UserID":7,
    "TailscaleIPs":["100.64.0.3"],"Online":false,"Relay":"hel","LastSeen":"2026-11-25T11:30:00+01:00"},
  "k4":{"HostName":"alpha","DNSName":"alpha.ex.ts.net.","OS":"linux","UserID":7,
    "TailscaleIPs":["100.64.0.4"],"Online":true,"CurAddr":"","Relay":"ams"},
  "k5":{"HostName":"friend","DNSName":"friend.ex.ts.net.","OS":"linux","UserID":99,
    "TailscaleIPs":["100.64.0.9"],"Online":true}}}
JSON
cat > "$T/keys.json" <<'JSON'
{"version":1,"entries":[
 {"keys":"super+Return","label":"Super+Enter","mods":64,"keysym":65293,"action":"terminal","run":null,
  "description":"Open a terminal","group":"Apps and launchers","source":"niruwm","app":null,"conflicts":[]},
 {"keys":"super+q","label":"Super+Q","mods":64,"keysym":113,"action":"close","run":null,
  "description":"Close the focused window","group":"Focus and windows","source":"niruwm","app":null,"conflicts":[]},
 {"keys":"super+shift+s","label":"Super+Shift+S","mods":65,"keysym":115,"action":"screenshot region","run":null,
  "description":"Take a screenshot of a region that you choose with the pointer and save it in the Pictures folder","group":"Screenshots and color","source":"niruwm","app":null,"conflicts":[]}],
 "problems":[]}
JSON
printf 'terminal = ["fake-term"]\n' > "$T/home/.config/nirulauncher/config.toml"

Xvfb :91 -screen 0 1920x1080x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X91 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:91 HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_DATA_HOME="$T/data" \
  XDG_DATA_DIRS="$T/sysdata" XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/cache" \
  XDG_RUNTIME_DIR="$T/run" NIRU_UI_SCALE=1 NIRUPANEL_DEBUG=hits
FULL_PATH="$T/bin:/usr/bin:/bin"

now_ms() { echo $(($(date +%s%N) / 1000000)); }
window() { xdotool search --name '^NIRUPANEL$' 2>/dev/null | head -1; }
last_frame() { grep 'nirupanel: frame' "$T/err" | tail -1; }
# Wait until the last frame matches a pattern (a grep -E expression).
wait_frame() {
  start=$(now_ms)
  until last_frame | grep -Eq "$1"; do
    [ $(($(now_ms) - start)) -lt 5000 ] || fail "no frame with $1"
    sleep 0.05
  done
}
wait_log() {
  start=$(now_ms)
  until [ -f "$T/$1" ] && grep -Fq -- "$2" "$T/$1"; do
    [ $(($(now_ms) - start)) -lt 3000 ] || fail "$1 never got '$2'"
    sleep 0.05
  done
}
# The middle of a target's rectangle: "x y".
centre() {
  last_frame | tr ' ' '\n' | grep -F "$1@" | head -1 | sed 's/.*@//' |
    awk -F, '{ printf "%d %d\n", $1 + $3 / 2, $2 + $4 / 2 }'
}
open_panel() {
  : > "$T/err"
  start=$(now_ms)
  PATH="$1" "$BIN" --panel 2>"$T/err" &
  PANEL=$!
  until grep -q 'nirupanel: frame' "$T/err" 2>/dev/null; do
    [ $(($(now_ms) - start)) -lt 3000 ] || fail "no first frame"
    kill -0 "$PANEL" 2>/dev/null || fail "the panel exited"
  done
  FIRST=$(($(now_ms) - start))
  until [ -n "$(window)" ]; do sleep 0.02; done
  WID=$(window)
  xdotool windowfocus --sync "$WID" 2>/dev/null || :
}
closed() {
  start=$(now_ms)
  while kill -0 "$PANEL" 2>/dev/null; do
    [ $(($(now_ms) - start)) -lt 3000 ] || fail "the panel did not close: $1"
    sleep 0.05
  done
  wait "$PANEL" 2>/dev/null || :
  PANEL=""
}
click() {
  at=$(centre "$2")
  [ -n "$at" ] || fail "no target $2"
  xdotool mousemove --window "$WID" "${at% *}" "${at#* }" click "$1"
}

# 1 and 2: the first frame while Tailscale is slow, then the computers.
export TS_SLEEP=1
open_panel "$FULL_PATH"
[ "$FIRST" -le "$BUDGET_MS" ] || fail "first frame after ${FIRST} ms (budget ${BUDGET_MS} ms)"
last_frame | grep -q 'Computer(' && fail "computers before Tailscale answered"
echo "PASS first frame in ${FIRST} ms while tailscale takes 1 s"
wait_frame 'Computer\(4\)'
last_frame | grep -q 'Computer(5)' && fail "a peer of another user is shown"
last_frame | tr ' ' '\n' | grep '@' | awk -F'@' -v frame="$(last_frame | sed 's/.*frame \([0-9]*x[0-9]*\).*/\1/')" '
  BEGIN { split(frame, wh, "x") }
  { split($2, r, ","); n++; x[n] = r[1]; y[n] = r[2]; w[n] = r[3]; h[n] = r[4]; name[n] = $1
    if (r[1] < 0 || r[2] < 0 || r[1] + r[3] > wh[1] || r[2] + r[4] > wh[2]) { print "outside: " $0; bad = 1 } }
  END {
    for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) {
      # Cards (Today, System) hold their own buttons; others must not meet.
      if (name[i] ~ /^(Today|System)$/ || name[j] ~ /^(Today|System)$/) continue
      if (x[i] < x[j] + w[j] && x[j] < x[i] + w[i] && y[i] < y[j] + h[j] && y[j] < y[i] + h[i]) {
        print "overlap: " name[i] " " name[j]; bad = 1 }
    }
    exit bad
  }' || fail "targets overlap or leave the window"
echo "PASS five own computers shown, nothing overlaps"
export TS_SLEEP=0

# 3: copy, then ssh and files.
click 3 'Computer(2)'
wait_log clipboard 100.64.0.2
kill -0 "$PANEL" 2>/dev/null || fail "copying closed the panel"
xdotool key ctrl+shift+c
wait_log clipboard othala
echo "PASS right-click copies the address, Ctrl+Shift+C the name"

# 4: key bindings, read once however much is typed.
xdotool key t e r m i n a l space o p e n
wait_frame 'Key\('
[ "$(grep -c '^keys --json' "$T/niructl.log")" -eq 1 ] || fail "niructl keys --json was not read exactly once"
xdotool key Return
closed "Enter on a key binding"
grep -qx 'keys run super+Return' "$T/niructl.log" || fail "the key binding did not run"
echo "PASS typing finds a key binding and Enter runs it"

open_panel "$FULL_PATH"
wait_frame 'Computer\(2\)'
click 1 'Computer(2)'
closed "ssh"
grep -q 'ssh othala$' "$T/fake-term.log" || fail "no ssh othala in the terminal: $(cat "$T/fake-term.log")"
echo "PASS a click runs ssh in the terminal"

open_panel "$FULL_PATH"
wait_frame 'Computer\(3\)'
click 2 'Computer(3)'
closed "files"
grep -qx 'sftp://mac/' "$T/xdg-open.log" || fail "no sftp://mac/"
echo "PASS a middle-click opens sftp://NAME/"

open_panel "$FULL_PATH"
xdotool key c l o s e space w i n d o w
wait_frame 'Key\('
xdotool key Return
sleep 0.3
grep -q 'keys run super+q' "$T/niructl.log" && fail "closing a window ran without asking"
kill -0 "$PANEL" 2>/dev/null || fail "the panel closed before asking"
xdotool key Return
closed "confirmed key binding"
grep -qx 'keys run super+q' "$T/niructl.log" || fail "the confirmed key binding did not run"
echo "PASS a key binding that closes a window asks once more"

# 5: the modules left out, or no Tailscale: nothing runs for them.
: > "$T/tailscale.log"
: > "$T/niructl.log"
printf 'terminal = ["fake-term"]\n[panel]\nmodules = ["apps", "power"]\n' \
  > "$T/home/.config/nirulauncher/config.toml"
open_panel "$FULL_PATH"
xdotool key t e r m
sleep 1
[ ! -s "$T/tailscale.log" ] || fail "tailscale ran without the computers module"
grep -q 'keys' "$T/niructl.log" && fail "niructl keys ran without the keys module"
xdotool key Escape Escape
closed "Escape"
printf 'terminal = ["fake-term"]\n' > "$T/home/.config/nirulauncher/config.toml"
open_panel "$T/bin-plain"
sleep 1
last_frame | grep -q 'Computer(' && fail "computers without Tailscale"
xdotool key Escape
closed "Escape"
echo "PASS nothing runs for modules that are off or without Tailscale"
