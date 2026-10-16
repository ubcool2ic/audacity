#!/usr/bin/env python3
"""Extra-menu commands 1-45 (docs/SPLIT_extra.md) vs Audacity 3.7.3 over mod-script-pipe.
Only pipe-observable behaviour is compared: the resulting selection (read back via AddLabel +
GetInfo Labels), clip positions (GetInfo Clips) and Snap-To preferences. Play/tool/volume/speed
commands have no pipe-visible effect and are covered by extra_cmds.rs unit tests."""
import os, sys, time, subprocess
sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
from compare_prefs import Ours, jload, body, R

SEL_CASES = [  # (start, end, command)
    (2.0, 4.0, "SelStart"), (2.0, 4.0, "SelEnd"),
    (2.0, 4.0, "SelExtLeft"), (2.0, 4.0, "SelExtRight"),
    (2.0, 4.0, "SelSetExtLeft"), (3.0, 3.0, "SelExtLeft"), (3.0, 3.0, "SelExtRight"),
    (0.0, 4.0, "SelExtLeft"), (6.0, 7.0, "SelExtRight"), (3.0, 3.0, "SelSetExtLeft"),
    (2.0, 4.0, "DeleteKey"), (2.0, 4.0, "Delete"), (0.0, 2.0, "ClipLeft"), (0.0, 2.0, "Trim"),
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
    def report(name, ok, detail):
        res.append(ok); print(f"{'PASS' if ok else 'FAIL'} {name:34s} {detail}", flush=True)
    with C.Aud() as A:
        for s, e, cmd in SEL_CASES:
            a = selection(A, s, e, cmd)
            with Ours() as O: o = selection(O, s, e, cmd)
            ok = bool(a[0] and o[0]) and abs(a[0][0]-o[0][0]) < 2e-3 and abs(a[0][1]-o[0][1]) < 2e-3 and a[1] == o[1]
            o = ((round(o[0][0], 4), round(o[0][1], 4)) if o[0] else None, o[1], o[2])
            report(f"{cmd} [{s},{e}]", ok, f"aud sel {a[0]} clips {a[1]} | ours sel {o[0]} clips {o[1]} {o[2][:60]!r}")
        for seq in (["ClipRight"] * 3, ["ClipRight"] * 3 + ["ClipLeft"], ["ClipLeft"], ["ClipLeft"] * 3 + ["ClipRight"] * 5):
            out = []
            for X in (A, None):
                ctx = Ours() if X is None else None
                if ctx: X = ctx.__enter__()
                setup(X); X("SelectTime: Start=0 End=10"); X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                for c in seq: X(f"{c}:")
                out.append([round(c["start"], 4) for c in jload(X("GetInfo: Type=Clips Format=JSON"))])
                if ctx: ctx.__exit__(None, None, None)
            report("+".join(seq), out[0] == out[1], f"aud {out[0]} ours {out[1]}")
        with Ours() as O:
            for cmd in ("SnapToOff", "SnapToNearest", "SnapToPrior", "SnapToOff"):
                out = []
                for X in (A, O):
                    X(f"{cmd}:"); out.append(body(X("GetPreference: Name=/Snap/Mode")).strip())
                report(cmd, out[0] == out[1], f"aud {out[0]} ours {out[1]}")
        # .aup3 with a clip before time 0, both directions.
        import glob
        for f in glob.glob("/tmp/au-xneg-*.aup3*"): os.remove(f)
        a_file, o_file = "/tmp/au-xneg-aud.aup3", "/tmp/au-xneg-ours.aup3"
        setup(A); A("SelectTime: Start=0 End=2"); A("SelectTracks: Track=0 TrackCount=1 Mode=Set")
        A("ClipLeft:"); A("ClipLeft:"); A(f"SaveProject2: Filename={a_file}")
        aud_clips = [round(c["start"], 4) for c in jload(A("GetInfo: Type=Clips Format=JSON"))]
        with Ours() as O:
            setup(O); O("SelectTime: Start=0 End=2"); O("ClipLeft:"); O("ClipLeft:"); O(f"SaveProject2: Filename={o_file}")
        with Ours() as O:
            O(f"OpenProject2: Filename={a_file}")
            ours_read = [round(c["start"], 4) for c in jload(O("GetInfo: Type=Clips Format=JSON"))]
        report("ours opens Audacity .aup3", ours_read == aud_clips, f"aud {aud_clips} ours {ours_read}")
    for f in glob.glob(o_file + "-*"): os.remove(f)
    with C.Aud() as A:  # fresh, empty Audacity so the opened file is the only project
        r = body(A(f"OpenProject2: Filename={o_file} AddToHistory=0"))
        aud_read = [round(c["start"], 4) for c in jload(A("GetInfo: Type=Clips Format=JSON"))]
        report("Audacity opens our .aup3", aud_read == aud_clips and "Failed" not in r, f"aud {aud_read} expected {aud_clips} {r!r}")
    print(f"{sum(res)}/{len(res)} passed")
    return 0 if all(res) else 1

if __name__ == "__main__":
    sys.exit(main())
