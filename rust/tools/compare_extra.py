import json
import os
import sys
import time
import subprocess

sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
import compare_prefs as P

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def body(r):
    return "\n".join(
        line for line in r.splitlines()
        if "BatchCommand finished:" not in line
    )


def setup(X, clear=False):
    if clear:
        X("SelectAll:")
        X("RemoveTracks:")

    for _ in range(3):
        X("NewMonoTrack:")
    X("Select: Start=0 End=10 Mode=Set")
    X("SelectTracks: Track=0 TrackCount=3 Mode=Set")
    X("Tone: Frequency=440 Amplitude=0.5")
    for i, name in enumerate(("A", "B", "C")):
        X(f"SetTrack: Track={i} Name={name}")
    X("ZoomNormal:")

    # Labels via Import2 of an Audacity label file (our pipe has no AddLabel).
    with open("/tmp/xcmp_labels.txt", "w") as f:
        f.write("2\t3\tone\n5\t5\ttwo\n")
    if isinstance(X, P.Ours):
        X("Import2: Filename=/tmp/xcmp_labels.txt")
    else:
        for i, (a, b, t) in enumerate(((2, 3, "one"), (5, 5, "two"))):
            X(f"Select: Start={a} End={b} Mode=Set")
            X("AddLabel:")
            X(f"SetLabel: Label={i} Text={t}")


def sel(X):
    if isinstance(X, P.Ours):
        info = json.loads(body(X("GetInfo: Type=Selection Format=JSON")))
        return (round(info["Start"], 4), round(info["End"], 4))
    X("AddLabel:")
    r = X("GetInfo: Type=Labels Format=JSON")
    labels = json.loads(body(r))
    empty = [
        (start, end)
        for entry in labels
        for start, end, text in entry[1]
        if text == ""
    ]
    if not empty:
        raise ValueError("No newly added empty label found")
    result = tuple(round(value, 4) for value in empty[-1])
    X("Undo:")
    return result


def tracks(X):
    r = X("GetInfo: Type=Tracks Format=JSON")
    data = json.loads(body(r))
    return [
        (
            entry["name"],
            round(entry["pan"], 3),
            round(entry["volume"], 3),
            bool(entry["mute"]),
            bool(entry["solo"]),
        )
        for entry in data
        if entry.get("kind") == "wave"
    ]


def main():
    subprocess.run(
        ["cargo", "build", "-q", "-p", "au-app", "--features", "gui"],
        cwd=R,
        check=True,
    )

    results = []
    PIXEL_CASES = {"CursorLeft point", "SelCntrLeft", "SelCntrRight", "SelSetExtRight"}

    def check(name, a, o):
        passed = a == o
        results.append(passed)
        print(f"{'PASS' if passed else 'FAIL'} {name}: Audacity={a!r} Ours={o!r}")

    with C.Aud() as A, P.Ours() as O:
        # Identical start state: Audacity builds it and saves an .aup3 that our app opens.
        setup(A, clear=True)
        if os.path.exists("/tmp/xcmp.aup3"):
            os.remove("/tmp/xcmp.aup3")
        A("SaveProject2: Filename=/tmp/xcmp.aup3")
        O("Import2: Filename=/tmp/xcmp.aup3")
        A("ZoomNormal:")

        time_cases = [
            ("CursorLeft", 2, 4, ["CursorLeft:"]),
            ("CursorRight", 2, 4, ["CursorRight:"]),
            ("CursorLeft point", 2, 2, ["CursorLeft:"]),
            ("CursorShortJumpRight", 2, 2, ["CursorShortJumpRight:"]),
            ("CursorShortJumpLeft sel", 2, 4, ["CursorShortJumpLeft:"]),
            ("CursorLongJumpRight clamp", 1, 1, ["CursorLongJumpRight:"]),
            ("CursorLongJumpLeft clamp", 3, 3, ["CursorLongJumpLeft:"]),
            ("SelCntrLeft", 2, 4, ["SelCntrLeft:"]),
            ("SelCntrRight", 2, 4, ["SelCntrRight:"]),
            ("SelSetExtRight", 2, 4, ["SelSetExtRight:"]),
            ("MoveToNextLabel", 0, 0, ["MoveToNextLabel:"]),
            ("MoveToNextLabel x2", 0, 0, ["MoveToNextLabel:", "MoveToNextLabel:"]),
            ("MoveToPrevLabel", 9, 9, ["MoveToPrevLabel:"]),
        ]

        for name, start, end, commands in time_cases:
            try:
                values = []
                for X in (A, O):
                    X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                    X(f"Select: Start={start} End={end} Mode=Set")
                    time.sleep(0.2)
                    statuses = []
                    for command in commands:
                        response = X(command)
                        statuses.append(
                            response.rstrip().endswith("BatchCommand finished: OK")
                        )
                        time.sleep(0.2)
                    t = sel(X)
                    if name in PIXEL_CASES:
                        # ZoomNormal is not implemented in our app yet (not in the BizyBot half),
                        # so compare 1-pixel moves in pixels: Audacity 44100/512 px/s, ours 100 px/s.
                        pps = 44100 / 512 if X is A else 100.0
                        t = tuple(round((v - b) * pps) for v, b in zip(t, (start, end)))
                    values.append((t, statuses))
                check(name, values[0], values[1])
            except TimeoutError as exc:
                check(name, f"TimeoutError: {exc}", None)

        track_cases = [
            ("TrackPanLeft x2", ["TrackPanLeft:", "TrackPanLeft:"]),
            ("TrackPanRight", ["TrackPanRight:"]),
            ("TrackGainInc x3", ["TrackGainInc:"] * 3),
            ("TrackGainDec", ["TrackGainDec:"]),
            ("TrackMute", ["TrackMute:"]),
            ("TrackSolo", ["TrackSolo:"]),
            ("TrackMoveUp", ["TrackMoveUp:"]),
            ("TrackMoveDown", ["TrackMoveDown:"]),
            ("TrackMoveTop", ["TrackMoveTop:"]),
            ("TrackMoveBottom", ["TrackMoveBottom:"]),
            ("TrackClose", ["TrackClose:"]),
        ]

        for name, commands in track_cases:
            try:
                values = []
                for X in (A, O):
                    X("SelectTracks: Track=1 TrackCount=1 Mode=Set")
                    X("SetTrack: Track=1 Focused=1")
                    for command in commands:
                        X(command)
                    values.append(tracks(X))
                check(name, values[0], values[1])
            except TimeoutError as exc:
                check(name, f"TimeoutError: {exc}", None)

        try:
            audio = []
            for X in (A, O):
                X("NewMonoTrack:")
                X("NewMonoTrack:")
                X("Select: Start=0 End=1 Mode=Set")
                n = len(tracks(X))
                X(f"SelectTracks: Track={n - 2} TrackCount=2 Mode=Set")
                X("Tone: Frequency=440 Amplitude=0.5")
                audio.append(body(X("CompareAudio: Threshold=0")).strip())
            # Track names differ between the two setups; compare the text with names masked.
            import re
            audio = [re.sub(r"'[^']*'", "'*'", a) for a in audio]
            check("CompareAudio", audio[0], audio[1])
        except TimeoutError as exc:
            check("CompareAudio", f"TimeoutError: {exc}", None)

    passed = sum(results)
    print(f"{passed}/{len(results)} passed")
    if passed != len(results):
        sys.exit(1)


if __name__ == "__main__":
    main()
