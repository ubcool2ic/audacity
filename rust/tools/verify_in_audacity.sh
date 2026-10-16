#!/bin/bash
# Opens each Audacity-made fixture and its Rust round-tripped copy in real
# Audacity 3.x, exports both to WAV from Audacity, and compares the exports and
# the track/clip/label info Audacity reports. Requires audacity + xvfb.
set -e
R=$(realpath "$(dirname "$0")/..")
W=${1:-/tmp/aup3_verify}; rm -rf "$W"; mkdir -p "$W"
P="python3 $R/tools/aud_pipe.py"
(cd "$R" && cargo build -q -p au-project --example aup3_tool)
T="$R/target/debug/examples/aup3_tool"
CFG=~/.config/audacity/audacity.cfg
# Share the box: serialize with other pipe users (same lock as compare_*.py) and only
# ever kill the Audacity this script started (its own process group), never `pkill -x audacity`.
exec 9>/tmp/audacity_script_pipe.lock; flock 9
AUD_PGID=
start() { # forget sessions killed earlier so the startup recovery dialog stays away
  python3 - "$CFG" <<'PY'
import re,sys
p=sys.argv[1]; s=open(p).read()
s=re.sub(r'\[ActiveProjects\]\n(?:[^\[\n][^\n]*\n)*', '', s)
open(p,'w').write(s)
PY
  rm -rf /var/tmp/audacity-$(id -un)/*
  setsid bash -c 'cd /tmp && exec xvfb-run -a -s "-screen 0 1280x1024x24" audacity' >"$W/aud.log" 2>&1 9>&- &
  AUD_PGID=$!
  for i in $(seq 60); do [ -p /tmp/audacity_script_pipe.to.$(id -u) ] && sleep 3 && return; sleep 1; done; exit 1; }
stop() {
  [ -n "$AUD_PGID" ] || return 0
  python3 $R/tools/aud_pipe.py "Close:" >/dev/null 2>&1 & sleep 3
  kill -- -"$AUD_PGID" 2>/dev/null || true; sleep 2; kill -9 -- -"$AUD_PGID" 2>/dev/null || true
  AUD_PGID=; rm -f /tmp/audacity_script_pipe.to.$(id -u) /tmp/audacity_script_pipe.from.$(id -u); }
probe() { # file tag
  start
  $P "OpenProject2: Filename=$1 AddToHistory=0" "SelectAll:" \
     "Export2: Filename=$W/$2.wav NumChannels=2" \
     "GetInfo: Type=Tracks Format=JSON" "GetInfo: Type=Clips Format=JSON" "GetInfo: Type=Labels Format=JSON" \
     "GetInfo: Type=Envelopes Format=JSON" 2> "$W/$2.info"
  stop
  # keep only info payloads, drop volatile fields
  grep -v "BatchCommand\|^>>" "$W/$2.info" | grep -v '"focused"' | grep -v '^Exported to ' > "$W/$2.json" || true
}
stop
fail=0
for f in "$R"/tests/fixtures/aup3/*.aup3; do
  b=$(basename "$f" .aup3); cp "$f" "$W/$b.orig.aup3"
  "$T" roundtrip "$f" "$W/$b.rust.aup3"
  if [ "$b" = crash_unsaved_edit ]; then
    # The Audacity-made crash fixture holds an orphan block from the killed
    # session, so Audacity shows a modal "not saved properly / recovered"
    # notice that blocks the pipe. Only the Rust copy (orphans dropped) is probed.
    probe "$W/$b.rust.aup3" "$b.rust"
    [ -s "$W/$b.rust.wav" ] && echo "PASS open $b (rust copy opens cleanly)" || { echo "FAIL open $b"; fail=1; }
    continue
  fi
  probe "$W/$b.orig.aup3" "$b.orig"; probe "$W/$b.rust.aup3" "$b.rust"
  if cmp -s "$W/$b.orig.wav" "$W/$b.rust.wav" && [ -s "$W/$b.rust.wav" ]; then echo "PASS wav  $b"; else echo "FAIL wav  $b"; fail=1; fi
  if diff -q "$W/$b.orig.json" "$W/$b.rust.json" >/dev/null; then echo "PASS info $b"; else echo "DIFF info $b"; diff "$W/$b.orig.json" "$W/$b.rust.json" | head -20; fi
done
exit $fail
