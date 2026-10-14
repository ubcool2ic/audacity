#!/bin/bash
# open.sh <aup3> <resave.aup3>
R=/workspace/audacity-bizybot/rust; export DISPLAY=:131
/workspace/audacity-official/run.sh || exit 1
( for i in $(seq 30); do for w in $(xdotool search --name "Project update required|Welcome to Audacity" 2>/dev/null); do xdotool windowactivate --sync $w key Return 2>/dev/null; done; sleep 1; done ) & K=$!
timeout 60 python3 $R/tools/aud_pipe.py "OpenProject2: Filename=$1 AddToHistory=0" "GetInfo: Type=Tracks Format=JSON" "SaveCopy: Filename=$2"
echo "alive: $(pgrep -f squashfs-root/bin/audacity | wc -l)"
kill $K 2>/dev/null; pkill -f squashfs-root/bin/audacity; sleep 2; rm -f /tmp/audacity_script_pipe.*
