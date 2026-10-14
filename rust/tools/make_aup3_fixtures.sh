#!/bin/bash
# Regenerates real Audacity 3.x .aup3 fixtures headlessly (xvfb + mod-script-pipe).
# Requires: apt install audacity xvfb ; mod-script-pipe=1 in ~/.config/audacity/audacity.cfg
set -e
OUT=$(realpath "${1:-$(dirname "$0")/../tests/fixtures/aup3}")
P="python3 $(dirname "$(realpath "$0")")/aud_pipe.py"
start() { (cd /tmp && xvfb-run -a -s "-screen 0 1280x1024x24" audacity >/tmp/aud.log 2>&1 &);
  for i in $(seq 60); do [ -p /tmp/audacity_script_pipe.to.$(id -u) ] && sleep 3 && return; sleep 1; done; echo no pipe; exit 1; }
stop() { pkill -x audacity || true; sleep 2; rm -f /tmp/audacity_script_pipe.*; }
CFG=~/.config/audacity/audacity.cfg
# sample format only applies to new projects, so set it in the cfg before launch
fmt() { sed -i "s/^DefaultProjectSampleFormatChoice=.*/DefaultProjectSampleFormatChoice=$1/" $CFG; }
stop
# A: int16 mono 44100, two clips, envelope, mix settings, labels, selection
fmt Format16Bit; start
$P "SetProject: Rate=44100" "NewMonoTrack:" "SelectTime: Start=0 End=1 RelativeTo=ProjectStart" \
   "Tone: Frequency=440 Amplitude=0.5 Waveform=Sine" \
   "SelectTime: Start=2 End=2.5 RelativeTo=ProjectStart" "Tone: Frequency=880 Amplitude=0.25 Waveform=Square" \
   "SelectTracks: Track=0 TrackCount=1 Mode=Set" \
   "SetTrack: Track=0 Name=Sine" "SetTrackAudio: Track=0 Gain=-6 Pan=25 Solo=1" \
   "SetEnvelope: Time=0.5 Value=0.5" "SetEnvelope: Time=0.9 Value=1.0" \
   "SelectTime: Start=0 End=0.5 RelativeTo=ProjectStart" "AddLabel:" "SetLabel: Label=0 Text=intro" \
   "SelectTime: Start=2.2 End=2.2 RelativeTo=ProjectStart" "AddLabel:" "SetLabel: Label=1 Text=point" \
   "SelectTracks: Track=0 TrackCount=1 Mode=Set" "SelectTime: Start=0.25 End=0.75 RelativeTo=ProjectStart" \
   "SaveProject2: Filename=$OUT/int16_mono_clips_labels.aup3"
stop
# B: float stereo 48000, noise, muted
fmt Format32BitFloat; start
$P "SetProject: Rate=48000" "NewStereoTrack:" "SelectTime: Start=0 End=2 RelativeTo=ProjectStart" \
   "Noise: Type=White Amplitude=0.3" "SetTrack: Track=0 Name=Noise" "SetTrackAudio: Track=0 Mute=1 Pan=-50" \
   "SaveProject2: Filename=$OUT/float_stereo_48k.aup3"
stop
# C: int24 mono 44100, 7 s chirp (spans several sample blocks)
fmt Format24Bit; start
$P "SetProject: Rate=44100" "NewMonoTrack:" "SelectTime: Start=0 End=7 RelativeTo=ProjectStart" \
   "Chirp: StartFreq=100 EndFreq=4000 StartAmp=0.8 EndAmp=0.1 Waveform=Sine" "SetTrack: Track=0 Name=Chirp" \
   "SaveProject2: Filename=$OUT/int24_mono_long.aup3"
stop
# D: crash fixture — save, then edit (Audacity journals the edit into the autosave table), then SIGKILL
fmt Format32BitFloat; start
$P "SetProject: Rate=44100" "NewMonoTrack:" "SelectTime: Start=0 End=1 RelativeTo=ProjectStart" \
   "Tone: Frequency=440 Amplitude=0.5" "SetTrack: Track=0 Name=Saved" \
   "SaveProject2: Filename=$OUT/crash_unsaved_edit.aup3" \
   "SelectTime: Start=1 End=1.5 RelativeTo=ProjectStart" "Tone: Frequency=220 Amplitude=0.4" "SetTrack: Track=0 Name=Unsaved"
sleep 2; pkill -9 -x audacity; sleep 1; rm -f /tmp/audacity_script_pipe.*
# fold WAL into the main files so fixtures are single self-contained files
for f in "$OUT"/*.aup3; do python3 -c "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('pragma wal_checkpoint(TRUNCATE)');c.execute('pragma journal_mode=DELETE');c.close()" "$f"; done
rm -f "$OUT"/*-wal "$OUT"/*-shm
ls -la "$OUT"
