#!/bin/sh
# File search under Xvfb with a fake home.
#
# 1. --list-files with each backend (the built-in walker, fd/fdfind when
#    installed, and plocate through a fake that reads a path list) finds
#    Fakturor/faktura-åäö.pdf, also for "faktura aao", and skips hidden
#    folders, node_modules and target.
# 2. --files: the empty field shows recently used files first
#    (recently-used.xbel); Enter opens the selected file with xdg-open,
#    Ctrl+Enter its folder, Shift+Enter and Ctrl+C copy the path.
# 3. "/ name" in the unified view searches files too.
#
# Usage: files.sh /path/to/nirulauncher
set -eu
BIN=${1:?usage: files.sh BIN}
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

H="$T/home"
mkdir -p "$T/bin" "$T/state" "$H/.config/nirulauncher" "$H/.local/share" \
  "$H/Fakturor/2024" "$H/.hidden" "$H/dev/app/node_modules/pkg" \
  "$H/dev/app/target/debug" "$H/dev/app/src" "$H/Bilder"
for f in Fakturor/faktura-åäö.pdf Fakturor/2024/mars.pdf .hidden/faktura-hemlig.pdf \
  dev/app/node_modules/pkg/faktura.js dev/app/target/debug/faktura \
  dev/app/src/faktura.rs Bilder/semester.jpg Bilder/kvitto.png; do
  : > "$H/$f"
done
cat > "$H/.local/share/recently-used.xbel" <<XBEL
<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0">
  <bookmark href="file://$H/Bilder/semester.jpg" added="2026-11-01T10:00:00Z" modified="2026-11-01T10:00:00Z" visited="2026-11-01T10:00:00Z"></bookmark>
  <bookmark href="file://$H/Bilder/kvitto.png" added="2026-11-08T10:00:00Z" modified="2026-11-08T10:00:00Z" visited="2026-11-08T10:00:00Z"></bookmark>
  <bookmark href="file://$H/gone.txt" added="2026-11-09T10:00:00Z"></bookmark>
</xbel>
XBEL
cat > "$T/bin/xdg-open" <<'FAKE'
#!/bin/sh
printf '%s\n' "xdg-open $*" >> "$LOG"
FAKE
# A plocate that answers from a path list, as the real one does from its
# database: `-l N --regex RE` and the `-l 1 -- PREFIX` coverage check.
cat > "$T/bin/plocate" <<'FAKE'
#!/bin/sh
if [ "${1:-}" = --version ]; then echo 'plocate 1.1.23 (fake)'; exit 0; fi
limit=100000 regex=0 out=$(mktemp)
while [ $# -gt 0 ]; do
  case $1 in
    -0|-i) ;;
    -l) shift; limit=$1 ;;
    --regex) regex=1 ;;
    --) shift; break ;;
    *) break ;;
  esac
  shift
done
cp "$LOCATE_DB" "$out"
# Every pattern must match, as in plocate.
for pattern in "$@"; do
  if [ "$regex" = 1 ]; then
    grep -E -i -- "$pattern" "$out" > "$out.n" || :
  else
    grep -F -- "$pattern" "$out" > "$out.n" || :
  fi
  mv "$out.n" "$out"
done
head -n "$limit" "$out" | tr '\n' '\000'
rm -f "$out"
FAKE
chmod +x "$T/bin/xdg-open" "$T/bin/plocate"
find "$H" > "$T/locate.db"

export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" \
  XDG_STATE_HOME="$T/state" LOG="$T/log" LOCATE_DB="$T/locate.db" \
  PATH="$T/bin:$PATH"
: > "$LOG"
CONFIG="$H/.config/nirulauncher/config.toml"

check_backend() {
  printf '[files]\nbackend = "%s"\n' "$1" > "$CONFIG"
  out=$("$BIN" --list-files faktura 2>"$T/err") || fail "$1: --list-files"
  echo "$out" | grep -q "^PDF	$H/Fakturor/faktura-åäö.pdf$" || fail "$1: no åäö file: $out"
  echo "$out" | grep -q "dev/app/src/faktura.rs" || fail "$1: no src file: $out"
  if echo "$out" | grep -q -e '\.hidden' -e node_modules -e target; then
    fail "$1: hidden or excluded path shown: $out"
  fi
  [ "$(echo "$out" | head -1 | cut -f2)" = "$H/Fakturor/faktura-åäö.pdf" ] ||
    [ "$(echo "$out" | head -1 | cut -f2)" = "$H/dev/app/src/faktura.rs" ] ||
    fail "$1: a folder match ranks first: $out"
  out=$("$BIN" --list-files faktura aao 2>"$T/err") || fail "$1: folded"
  [ "$(echo "$out" | cut -f2)" = "$H/Fakturor/faktura-åäö.pdf" ] ||
    fail "$1: 'faktura aao' should find only the åäö file: $out"
  out=$("$BIN" --list-files 2024 fakturor 2>"$T/err") || fail "$1: folders"
  echo "$out" | grep -q "Fakturor/2024/mars.pdf" || fail "$1: words in folder names: $out"
  grep -q "$2" "$T/err" || fail "$1: backend note '$(cat "$T/err")', want $2"
  echo "PASS $1: finds the files, folds åäö, skips hidden, node_modules and target"
}
check_backend builtin 'built-in'
check_backend plocate plocate
if command -v fdfind >/dev/null 2>&1; then
  check_backend fdfind fdfind
elif command -v fd >/dev/null 2>&1; then
  check_backend fd 'fd'
else
  echo 'SKIP fd: not installed'
fi

printf '[files]\nbackend = "builtin"\n' > "$CONFIG"
Xvfb :95 -screen 0 1280x800x24 -nolisten tcp -noreset >"$T/xvfb.log" 2>&1 &
XVFB_PID=$!
i=0
while [ ! -S /tmp/.X11-unix/X95 ]; do
  i=$((i + 1)); [ "$i" -gt 100 ] && { cat "$T/xvfb.log"; exit 1; }
  sleep 0.05
done
export DISPLAY=:95

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
  while ! grep -qxF "$1" "$LOG"; do
    i=$((i + 1)); [ "$i" -gt 60 ] && fail "$2: log is '$(cat "$LOG")', want '$1'"
    sleep 0.05
  done
}

open --files
xdotool key Return
closed 'recent enter'
logged "xdg-open $H/Bilder/kvitto.png" 'empty field'
echo 'PASS the empty field lists recent files, newest first; Enter opens it'

open --files
xdotool type --delay 20 'faktura aao'
sleep 0.6
xdotool key Return
closed 'search enter'
logged "xdg-open $H/Fakturor/faktura-åäö.pdf" 'search'
echo 'PASS typed words find the file; Enter opens it with xdg-open'

open --files
xdotool type --delay 20 'faktura aao'
sleep 0.6
xdotool key ctrl+Return
closed 'ctrl enter'
logged "xdg-open $H/Fakturor" 'Ctrl+Enter'
echo 'PASS Ctrl+Enter opens the containing folder'

open --files
xdotool type --delay 20 'faktura aao'
sleep 0.6
xdotool key shift+Return
closed 'shift enter'
i=0
while [ "$(xclip -selection clipboard -o 2>/dev/null)" != "$H/Fakturor/faktura-åäö.pdf" ]; do
  i=$((i + 1)); [ "$i" -gt 60 ] && fail "Shift+Enter: clipboard is '$(xclip -selection clipboard -o 2>/dev/null)'"
  sleep 0.05
done
echo 'PASS Shift+Enter copies the path'
pkill xclip 2>/dev/null || :

open --files
xdotool type --delay 20 'mars'
sleep 0.6
xdotool key ctrl+c
closed 'ctrl c'
i=0
while [ "$(xclip -selection clipboard -o 2>/dev/null)" != "$H/Fakturor/2024/mars.pdf" ]; do
  i=$((i + 1)); [ "$i" -gt 60 ] && fail "Ctrl+C: clipboard is '$(xclip -selection clipboard -o 2>/dev/null)'"
  sleep 0.05
done
echo 'PASS Ctrl+C copies the path too'
pkill xclip 2>/dev/null || :

open --apps
xdotool type --delay 20 '/ mars'
sleep 0.6
xdotool key Return
closed 'prefix'
logged "xdg-open $H/Fakturor/2024/mars.pdf" '/ prefix'
echo 'PASS "/ name" in the unified view searches files'

printf '[files]\nenabled = false\n' > "$CONFIG"
out=$("$BIN" --list-actions)
if echo "$out" | grep -q 'Find files'; then fail 'Find files listed while turned off'; fi
printf '' > "$CONFIG"
out=$("$BIN" --list-actions)
echo "$out" | grep -q 'Find files' || fail 'Find files is not a NIRU Action'
echo 'PASS Find files is a NIRU Action unless [files] enabled = false'
