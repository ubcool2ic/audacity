#!/bin/bash
# Start official Audacity 3.7.3 AppImage (has VST3) on Xvfb :131 + openbox, own HOME; wait for mod-script-pipe.
H=/workspace/audacity-official/home; export DISPLAY=:131
pkill -f squashfs-root/bin/audacity; sleep 1; rm -f /tmp/audacity_script_pipe.*
pgrep -f "Xvfb :131" >/dev/null || { Xvfb :131 -screen 0 1280x1024x24 >/dev/null 2>&1 & sleep 1; }
pgrep -x openbox >/dev/null || { openbox >/dev/null 2>&1 & sleep 1; }
python3 - $H/.config/audacity/audacity.cfg <<'PY'
import re,sys
p=sys.argv[1]; s=open(p).read(); s=re.sub(r'\[ActiveProjects\]\n(?:[^\[\n][^\n]*\n)*', '', s); open(p,'w').write(s)
PY
rm -rf /var/tmp/audacity-$(id -un)/*
(cd /tmp && HOME=$H /workspace/audacity-official/squashfs-root/AppRun >/tmp/aud373.log 2>&1 &)
for i in $(seq 120); do [ -p /tmp/audacity_script_pipe.to.$(id -u) ] && sleep 3 && exit 0; sleep 1; done; exit 1
