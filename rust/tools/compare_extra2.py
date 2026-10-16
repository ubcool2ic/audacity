"""Extra-menu items 68-89 (docs/SPLIT_extra.md) vs Audacity 3.7.3 over mod-script-pipe.
Started from BizyBot's unpushed compare_extra.py; items 46-67 cases removed."""
import json
import re
import os
import sys
import time
import subprocess

sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
import compare_prefs as P
import signal


class SafeAud(C.Aud):
    """C.Aud with retries. C.Aud.__enter__ now kills its own Audacity and releases the lock itself
    when startup fails, so a retry starts clean."""
    def __enter__(self):
        for attempt in range(3):
            try:
                return super().__enter__()
            except (OSError, TimeoutError, RuntimeError):
                time.sleep(5)
        raise RuntimeError("Audacity did not start")


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
    tone(X, 0.5)
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


def tone(X, amp):
    # Audacity's Tone keeps Chirp's last EndAmp (shared ToneGenBase settings), so generate a flat tone
    # with Chirp there; ours: Tone.
    if isinstance(X, P.Ours):
        X(f"Tone: Frequency=440 Amplitude={amp}")
    else:
        X(f"Chirp: StartFreq=440 EndFreq=440 StartAmp={amp} EndAmp={amp} Waveform=Sine Interpolation=Linear")


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

    with SafeAud() as A, P.Ours() as O:
        # Identical start state: Audacity builds it and saves an .aup3 that our app opens.
        setup(A, clear=True)
        if os.path.exists("/tmp/xcmp.aup3"):
            os.remove("/tmp/xcmp.aup3")
        A("SaveProject2: Filename=/tmp/xcmp.aup3")
        O("Import2: Filename=/tmp/xcmp.aup3")
        A("ZoomNormal:")

        time_cases = [
            ("CursorShortJumpRight", 2, 2, ["CursorShortJumpRight:"]),
            ("CursorShortJumpLeft sel", 2, 4, ["CursorShortJumpLeft:"]),
            ("CursorShortJumpRight sel", 2, 4, ["CursorShortJumpRight:"]),
            ("CursorShortJumpLeft point", 3, 3, ["CursorShortJumpLeft:"]),
            ("CursorLongJumpRight", 2, 2, ["CursorLongJumpRight:"]),
            ("CursorLongJumpLeft sel", 2, 4, ["CursorLongJumpLeft:"]),
            ("CursorLongJumpRight clamp", 1, 1, ["CursorLongJumpRight:"]),
            ("CursorLongJumpLeft clamp", 3, 3, ["CursorLongJumpLeft:"]),
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

        # 89 CompareAudio on the A/B tracks (identical tones, then B partly quieter).
        for name, prep, thr in (("CompareAudio equal", None, "0"), ("CompareAudio differing", "chirp", "0.005")):
            try:
                out = []
                for X in (A, O):
                    if prep:
                        X("Select: Start=0 End=1 Mode=Set"); X("SelectTracks: Track=1 TrackCount=1 Mode=Set"); tone(X, 0.25)
                    X("Select: Start=0 End=1 Mode=Set")
                    X("SelectTracks: Track=0 TrackCount=2 Mode=Set")
                    out.append(body(X(f"CompareAudio: Threshold={thr}")).strip())
                if prep:
                    # Our Tone and Audacity's Chirp differ by float rounding next to zero crossings,
                    # so allow 0.1% on the count and compare the message with counts masked.
                    n = [int(re.search(r"Finished comparison: (\d+)", o).group(1)) for o in out]
                    print("   counts", n)
                    out = [re.sub(r"[\d.]+", "#", o.split("Finished")[1]) if abs(n[0] - n[1]) <= max(n) / 1000 else o for o in out]
                check(name, out[0], out[1])
            except TimeoutError as exc:
                check(name, f"TimeoutError: {exc}", None)

        track_cases = [
            ("TrackPanLeft x2", ["TrackPanLeft:", "TrackPanLeft:"]),
            ("TrackPanRight", ["TrackPanRight:"]),
            ("TrackGainInc x3", ["TrackGainInc:"] * 3),
            ("TrackGainDec", ["TrackGainDec:"]),
            ("TrackMute", ["TrackMute:"]),
            ("TrackSolo", ["TrackSolo:"]),
            ("TrackSolo again", ["TrackSolo:"]),
            ("TrackMute again", ["TrackMute:"]),
            ("TrackGainDec x40", ["TrackGainDec:"] * 40),
            ("TrackPanRight x15", ["TrackPanRight:"] * 15),
            ("TrackMoveUp", ["TrackMoveUp:"]),
            ("TrackMoveDown", ["TrackMoveDown:"]),
            ("TrackMoveTop", ["TrackMoveTop:"]),
            ("TrackMoveBottom", ["TrackMoveBottom:"]),
            ("TrackClose", ["TrackClose:"]),
        ]

        for solo in ("Multi",):
          for X in (A, O):
              X(f"SetPreference: Name=/GUI/Solo Value={solo}")
          for name, commands in [(f"{n} [{solo}]", c) for n, c in track_cases if solo == "Multi" or "Solo" in n or "Mute" in n]:
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

        A("SetPreference: Name=/GUI/Solo Value=Multi")

    # Fresh sessions: Simple solo behaviour, CompareAudio with a difference, SetEnvelope/SetProject.
    time.sleep(10)
    with SafeAud() as A, P.Ours() as O:
        for X in (A, O):
            setup(X, clear=X is A)
            X("SetPreference: Name=/GUI/Solo Value=Simple")
        for name, commands in (("TrackMute [Simple]", ["TrackMute:"]), ("TrackSolo [Simple]", ["TrackSolo:"]),
                               ("TrackSolo again [Simple]", ["TrackSolo:"]), ("TrackMute x2 [Simple]", ["TrackMute:", "TrackMute:"])):
            values = []
            for X in (A, O):
                X("SelectTracks: Track=1 TrackCount=1 Mode=Set"); X("SetTrack: Track=1 Focused=1")
                for c in commands: X(c)
                values.append(tracks(X))
            check(name, [v[1:] for v in values[0]], [v[1:] for v in values[1]])
        A("SetPreference: Name=/GUI/Solo Value=Multi")
        import wave, struct
    def peak(path):
        w = wave.open(path); n = w.getnframes(); sw = w.getsampwidth(); rate = w.getframerate()
        data = w.readframes(n)
        if sw == 2: vals = [v / 32768 for v in struct.unpack(f"<{len(data)//2}h", data)]
        else: vals = [int.from_bytes(data[i:i+sw], "little", signed=True) / 2 ** (8 * sw - 1) for i in range(0, len(data), sw)]
        # 2 decimals: the sources differ (our Tone vs Audacity's Chirp, 16-bit dither); the exact
        # sample-by-sample SetEnvelope check on identical audio is in compare_extra2_close.py.
        return rate, round(max(abs(v) for v in vals[: rate]), 2), round(max(abs(v) for v in vals[rate * 3: rate * 4]), 2)
    for name, cmds in (
        ("SetEnvelope Time+Value", ["SetEnvelope: Time=2 Value=0.5"]),
        ("SetEnvelope 2 points", ["SetEnvelope: Time=0 Value=0.2", "SetEnvelope: Time=4 Value=1.5"]),
        ("SetEnvelope Delete", ["SetEnvelope: Time=2 Value=0.5", "SetEnvelope: Delete=1"]),
        ("SetEnvelope outside clip", ["SetEnvelope: Time=50 Value=0.1"]),
        ("SetProject Rate", ["SetProject: Rate=22050"]),
    ):
        try:
            out = []
            time.sleep(10)
            with SafeAud() as A, P.Ours() as O:
              for X in (A, O):
                X("SelectAll:"); X("RemoveTracks:")
                for c in cmds:
                    if c.startswith("SetProject"): X(c)
                X("NewMonoTrack:"); X("Select: Start=0 End=5 Mode=Set")
                X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                tone(X, 0.5)
                for c in cmds:
                    if c.startswith("SetEnvelope"): X(c)
                path = f"/tmp/xcmp2_{'a' if X is A else 'o'}.wav"
                if os.path.exists(path): os.remove(path)
                X("Select: Start=0 End=5 Mode=Set")
                X(f"Export2: Filename={path} NumChannels=1")
                time.sleep(1); out.append(peak(path))
            check(name, out[0], out[1])
        except Exception as exc:
            check(name, f"{type(exc).__name__}: {exc}", None)


    passed = sum(results)
    print(f"{passed}/{len(results)} passed")
    if passed != len(results):
        sys.exit(1)


if __name__ == "__main__":
    main()
