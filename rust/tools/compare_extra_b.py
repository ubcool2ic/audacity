#!/usr/bin/env python3
"""BizyBot Extra-menu items 46-67 (docs/SPLIT_extra.md) vs Audacity 3.7.3 over mod-script-pipe.
Reuses compare_extra.py's harness for the selection/cursor cases, then adds label navigation
(MoveToPrevLabel/MoveToNextLabel) and track-focus/toggle cases (First/Last/Prev/NextTrack,
ShiftUp/ShiftDown, Toggle/ToggleAlt), compared through the pipe-visible track selection.
Not pipe-comparable (see docs/SPLIT_extra.md notes): ExportMp3/Wav/Ogg/FLAC/Sel open a modal
export dialog in Audacity that blocks the pipe; PrevFrame/NextFrame only move keyboard focus
between toolbars and tracks. Track focus is compared directly via GetInfo "focused"."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
import compare_extra as X
import compare_effects as C
from compare_prefs import Ours, jload, body

X.SEL_CASES[:] = [
    (2.0, 4.0, "SelSetExtRight"), (3.0, 3.0, "SelSetExtRight"), (9.99, 10.0, "SelSetExtRight"),
    (2.0, 4.0, "SelCntrLeft"), (2.0, 4.0, "SelCntrRight"), (3.0, 3.0, "SelCntrLeft"),
    (2.0, 4.0, "CursorLeft"), (2.0, 4.0, "CursorRight"), (3.0, 3.0, "CursorLeft"),
    (3.0, 3.0, "CursorRight"), (0.0, 0.0, "CursorLeft"),
]

LABELS = [(1.0, 1.5), (4.0, 5.0), (7.0, 7.0)]
LABEL_CASES = [  # (cursor start, end, command sequence)
    (0.0, 0.0, ["MoveToNextLabel"]), (0.0, 0.0, ["MoveToNextLabel"] * 2),
    (3.0, 3.0, ["MoveToNextLabel"]), (3.0, 3.0, ["MoveToPrevLabel"]),
    (9.0, 9.0, ["MoveToPrevLabel"]), (9.0, 9.0, ["MoveToNextLabel"]),
    (0.5, 0.5, ["MoveToPrevLabel"]), (4.0, 5.0, ["MoveToNextLabel"]),
    (4.0, 5.0, ["MoveToPrevLabel"]), (0.0, 0.0, ["MoveToNextLabel"] * 3 + ["MoveToPrevLabel"]),
]
FOCUS_CASES = [
    ["FirstTrack", "Toggle"], ["LastTrack", "Toggle"], ["LastTrack", "ToggleAlt"],
    ["FirstTrack", "NextTrack", "Toggle"], ["LastTrack", "PrevTrack", "ToggleAlt"],
    ["FirstTrack", "PrevTrack", "Toggle"], ["LastTrack", "NextTrack", "Toggle"],
    ["FirstTrack", "ShiftDown"], ["FirstTrack", "ShiftDown", "ShiftDown"],
    ["LastTrack", "ShiftUp"], ["LastTrack", "ShiftUp", "ShiftUp", "ShiftDown"],
    ["FirstTrack", "ShiftUp"], ["LastTrack", "ShiftDown"], ["FirstTrack", "Toggle", "Toggle"],
]

def ours(X): return X.__class__.__name__ == "Ours"

def labels(X):
    labs = jload(X("GetInfo: Type=Labels Format=JSON"))
    return sorted((round(l[0], 4), round(l[1], 4)) for t in labs for l in t[1])

def label_case(X, s, e, seq):
    X_ = X
    globals()["X"].setup(X_)
    for a, b in LABELS:
        X_(f"SelectTime: Start={a} End={b}"); X_("AddLabel:")
    X_(f"SelectTime: Start={s} End={e}"); X_("SelectTracks: Track=0 TrackCount=1 Mode=Set")
    for c in seq: X_(f"{c}:")
    if ours(X_):
        v = [float(t) for t in body(X_("GetSelection:")).split()]
        return (round(v[0], 4), round(v[1], 4))
    before = labels(X_); X_("AddLabel:"); after = labels(X_)
    for l in before: after.remove(l)
    return after[0] if after else None

def focus_case(X, seq):
    if not ours(X): X("SelectAll:"); X("RemoveTracks:")
    for _ in range(3): X("NewMonoTrack:")
    X("SelectTime: Start=0 End=1"); X("SelectTracks: Track=1 TrackCount=1 Mode=Set")
    for c in seq: X(f"{c}:")
    tr = jload(X("GetInfo: Type=Tracks Format=JSON"))
    return [int(bool(t.get("selected"))) for t in tr], [int(bool(t.get("focused"))) for t in tr]

def main():
    rc = X.main()
    res = []
    def report(name, ok, detail):
        res.append(ok); print(f"{'PASS' if ok else 'FAIL'} {name:40s} {detail}", flush=True)
    with C.Aud() as A:
        for s, e, seq in LABEL_CASES:
            a = label_case(A, s, e, seq)
            with Ours() as O: o = label_case(O, s, e, seq)
            ok = a is not None and o is not None and abs(a[0]-o[0]) < 2e-3 and abs(a[1]-o[1]) < 2e-3
            report(f"{'+'.join(seq)} [{s},{e}]", ok, f"aud {a} ours {o}")
        for seq in FOCUS_CASES:
            a = focus_case(A, seq)
            with Ours() as O: o = focus_case(O, seq)
            report("+".join(seq), a == o, f"aud sel {a[0]} focus {a[1]} | ours sel {o[0]} focus {o[1]}")
    print(f"labels/focus: {sum(res)}/{len(res)} passed")
    return rc or (0 if all(res) else 1)

if __name__ == "__main__":
    sys.exit(main())
