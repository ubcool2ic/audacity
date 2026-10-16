#!/usr/bin/env python3
"""Compare Snap-To modes and targets vs Audacity 3.7.3 over mod-script-pipe."""
import os, sys, subprocess
sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
from compare_prefs import Ours, jload, body, R

SNAP_IDS = os.environ["SNAP_IDS"].split(",") if os.environ.get("SNAP_IDS") else [
    "seconds", "deciseconds", "milliseconds", "samples", "film_24_fps",
    "ntsc_29.97_fps", "film_25_fps", "cd_75_fps", "bar", "bar_1_8", "triplet_1_4",
]
MODES = ["NEAREST", "PRIOR"]
CASES = [
    (2.37, 2.37, "SelExtRight"), (2.37, 2.37, "SelExtLeft"),
    (1.234, 3.456, "SelExtRight"), (1.234, 3.456, "SelExtLeft"),
    (2.37, 2.37, "SelStart"), (2.37, 2.37, "SelEnd"),
]

def setup(X):
    if X.__class__.__name__ != "Ours": X("SelectAll:"); X("RemoveTracks:")
    X("NewMonoTrack:"); X("SelectTime: Start=0 End=10"); X('Tone: Frequency=440 Amplitude=0.5 Waveform="Sine"')
    X("ZoomNormal:")

def selection(X, s, e, cmd):
    setup(X)
    X(f"SelectTime: Start={s} End={e}"); X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
    r = body(X(f"{cmd}:"))
    if X.__class__.__name__ == "Ours":
        lab = [float(v) for v in body(X("GetSelection:")).split()]
    else:
        X("AddLabel:")
        labs = jload(X("GetInfo: Type=Labels Format=JSON"))
        lab = labs[-1][1][-1] if labs and labs[-1][1] else None
    clips = [(round(c["start"], 6), round(c["end"], 6)) for c in jload(X("GetInfo: Type=Clips Format=JSON"))]
    return (round(lab[0], 4), round(lab[1], 4)) if lab else None, clips, r

def main():
    subprocess.run(["cargo", "build", "-q", "-p", "au-app", "--features", "gui"], cwd=R, check=True)
    res = []
    cfg = os.path.expanduser("~/.config/audacity/audacity.cfg")
    import re
    for snap_to in SNAP_IDS:
        # ProjectSnap reads /Snap/To only at project creation, so start Audacity per id.
        t = re.sub(r"\[Snap\]\n(?:[^\[\n][^\n]*\n)*", "", open(cfg).read())
        open(cfg, "w").write(t + f"[Snap]\nTo={snap_to}\nMode=NEAREST\n")
        with C.Aud() as A, Ours() as O:
            O(f"SetPreference: Name=/Snap/To Value={snap_to} Reload=1")
            for mode in MODES:
                for X in (A, O):
                    X(f"SetPreference: Name=/Snap/Mode Value={mode} Reload=1")
                    X(f"SnapTo{mode.title()}:")
                for s, e, cmd in CASES:
                    a = selection(A, s, e, cmd)
                    o = selection(O, s, e, cmd)
                    ok = bool(a[0] and o[0]) and abs(a[0][0] - o[0][0]) <= 1.01e-4 and abs(a[0][1] - o[0][1]) <= 1.01e-4
                    res.append(ok)
                    print(f"{'PASS' if ok else 'FAIL'} {snap_to} {mode} {cmd} [{s},{e}] aud {a[0]} ours {o[0]}", flush=True)
            A("SnapToOff:")
    t = re.sub(r"\[Snap\]\n(?:[^\[\n][^\n]*\n)*", "", open(cfg).read())
    open(cfg, "w").write(t + "[Snap]\nTo=bar_1_8\nMode=OFF\n")
    print(f"{sum(res)}/{len(res)} match")
    sys.exit(0 if all(res) else 1)

if __name__ == "__main__": main()
