#!/bin/bash
# probe.sh <project.aup3> <out.wav> : open in official Audacity 3.7.3, auto-dismiss info dialogs, export mono WAV
R=/workspace/audacity-bizybot/rust
/workspace/audacity-official/run.sh || exit 1
export DISPLAY=:131
( for i in $(seq 60); do for w in $(xdotool search --name "Project update required|Welcome to Audacity|App updates|Warning|Error" 2>/dev/null); do xdotool windowactivate --sync $w key Return 2>/dev/null; done; sleep 1; done ) &
K=$!
timeout 90 python3 $R/tools/aud_pipe.py "OpenProject2: Filename=$1 AddToHistory=0" "GetInfo: Type=Tracks Format=JSON" "SelectAll:" "Export2: Filename=$2 NumChannels=1"
kill $K 2>/dev/null
pkill -f squashfs-root/bin/audacity; sleep 2; rm -f /tmp/audacity_script_pipe.*
