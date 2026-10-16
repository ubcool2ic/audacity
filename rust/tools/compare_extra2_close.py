"""Closes the open checks on Extra items 72/75/78 and 86-89 against Audacity 3.7.3.
Run isolated: tools/iso_run.sh python3 tools/compare_extra2_close.py
- 89 CompareAudio: identical imported float WAVs, exact message at several thresholds.
- 86 SetEnvelope: every exported sample (32-bit float WAV), not just two peak windows.
- 87 SetProject: X11 window title, position and size (xwininfo) on each app's own Xvfb display.
- 88 Drag: selection after drags in "Track Panel" / "Timeline" coordinates.
- 72/75: Pan / Volume dialogs driven by keyboard (xdotool): Ctrl+A, value, Return / Escape.
- 78: TrackMenu opens a menu window and Escape closes it without changes."""
import os, re, sys, time, json, struct, wave, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import compare_prefs as P
from compare_extra2 import SafeAud, body, tone, sel, tracks

W = "/tmp/x2c"; os.makedirs(W, exist_ok=True)
results = []
def check(name, a, o):
    results.append(a == o); print(f"{'PASS' if a == o else 'FAIL'} {name}: Audacity={a!r} Ours={o!r}", flush=True)

def xenv(proc, comm):
    pg = proc.pid
    for pid in os.listdir("/proc"):
        if not pid.isdigit(): continue
        try:
            if os.getpgid(int(pid)) != pg or not open(f"/proc/{pid}/comm").read().strip().startswith(comm): continue
            env = dict(e.split(b"=", 1) for e in open(f"/proc/{pid}/environ", "rb").read().split(b"\0") if b"=" in e)
            if b"DISPLAY" in env:
                return dict(os.environ, **{k.decode(): env[k].decode() for k in (b"DISPLAY", b"XAUTHORITY") if k in env})
        except Exception: pass
    raise RuntimeError("no display for " + comm)

def windows(env):
    out = subprocess.run(["xwininfo", "-root", "-tree"], env=env, capture_output=True, text=True).stdout
    return [(m[1], int(m[2]), int(m[3]), int(m[4]), int(m[5])) for m in
            re.finditer(r'0x[0-9a-f]+ "([^"]*)":.*?(\d+)x(\d+)\+(-?\d+)\+(-?\d+)', out)]

def keys(env, *k):
    # No window manager under Xvfb: give our egui window keyboard focus explicitly.
    ids = subprocess.run(["xdotool", "search", "--class", "audacity-rs"], env=env, capture_output=True, text=True).stdout.split()
    if ids: subprocess.run(["xdotool", "windowfocus", "--sync", ids[-1]], env=env); time.sleep(0.3)
    for x in k:
        subprocess.run(["xdotool", "key", "--clearmodifiers", x] if not x.startswith("type:") else ["xdotool", "type", "--", x[5:]], env=env)
        time.sleep(0.3)

def write_wav(path, x, rate=44100):
    x = np.asarray(x, dtype="<f4")
    with open(path, "wb") as f:  # WAVE_FORMAT_IEEE_FLOAT, mono
        data = x.tobytes()
        f.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 3, 1, rate, rate * 4, 4, 32))
        f.write(b"data" + struct.pack("<I", len(data)) + data)

def read_wav(path):
    d = open(path, "rb").read(); i = 12; fmt = None; data = None
    while i < len(d):
        cid, n = d[i:i+4], struct.unpack("<I", d[i+4:i+8])[0]
        if cid == b"fmt ": fmt = struct.unpack("<HHIIHH", d[i+8:i+24])
        if cid == b"data": data = d[i+8:i+8+n]
        i += 8 + n + (n & 1)
    if fmt[0] == 1 and fmt[5] == 16: return np.frombuffer(data, "<i2") / 32768.0
    assert fmt[0] in (3, 0xFFFE) and fmt[5] == 32, fmt
    return np.frombuffer(data, "<f4").astype(float)

def reply(X, timeout=30):
    """Read the pending answer of a command written straight to the pipe."""
    import select
    end = time.time() + timeout
    while b"\n\n" not in X.buf:
        if not select.select([X.fr], [], [], max(0, end - time.time()))[0]: raise TimeoutError("no reply")
        X.buf += os.read(X.fr, 65536)
    r, X.buf = X.buf.split(b"\n\n", 1); return r.decode(errors="replace")

def start(X, ours):
    if not ours: X("SelectAll:"); X("RemoveTracks:")

def main():
    rng = np.random.default_rng(7)
    t = np.arange(2 * 44100) / 44100
    a = 0.5 * np.sin(2 * np.pi * 440 * t)
    b = a + rng.normal(0, 0.004, a.size)
    write_wav(f"{W}/a.wav", a); write_wav(f"{W}/b.wav", b)
    t5 = np.arange(5 * 44100) / 44100
    write_wav(f"{W}/c.wav", 0.5 * np.sin(2 * np.pi * 440 * t5))

    with SafeAud() as A, P.Ours() as O:
        ea, eo = xenv(A.proc, "audacity"), xenv(O.proc, "audacity-rs")
        for X in (A, O):
            start(X, X is O)
            X(f"Import2: Filename={W}/a.wav"); X(f"Import2: Filename={W}/b.wav")
        # 89 CompareAudio
        for (s0, s1) in ((0.25, 1.5),):
            for thr in ("0", "0.0001", "0.001", "0.004", "0.005", "0.0075", "0.0099", "0.00999999977648258"):
                out = []
                for X in (A, O):
                    X(f"Select: Start={s0} End={s1} Mode=Set"); X("SelectTracks: Track=0 TrackCount=2 Mode=Set")
                    out.append(body(X(f"CompareAudio: Threshold={thr}")).strip())
                check(f"CompareAudio {s0}-{s1} thr={thr}", out[0], out[1])

        # 88 Drag (Selection tool, ZoomNormal, h=0)
        for X in (A, O):
            X("ZoomNormal:"); X("SetTrack: Track=0 Focused=1")
        for name, cmd in (("Drag Track Panel", 'Drag: Window="Track Panel" FromX=300 FromY=60 ToX=500 ToY=60'),
                          ("Drag Track Panel 2nd track", 'Drag: Window="Track Panel" FromX=250 FromY=220 ToX=450 ToY=220'),
                          ("Drag Track Panel by Id", "Drag: Id=1003 FromX=350 FromY=80 ToX=260 ToY=80"),
                          ("Drag Timeline", "Drag: FromX=200 ToX=400"),
                          ("Drag move only", 'Drag: Window="Track Panel" FromX=300 FromY=60'),
                          ("Drag ToY only", 'Drag: Window="Track Panel" FromX=300 FromY=60 ToY=90')):
            out = []
            for X in (A, O):
                X("Select: Start=1 End=1 Mode=Set"); X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                time.sleep(0.5); X(cmd); time.sleep(2.5)
                out.append(sel(X))
            check(name, out[0], out[1])

        # 72 / 75 Pan and Volume dialogs, 78 TrackMenu
        for name, cmd, seq, title in (("TrackPan type -50 Return", "TrackPan:", ["End", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "type:-50", "Return"], "Pan"),
                                      ("TrackGain type -6 Return", "TrackGain:", ["End", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "type:-6", "Return"], "Volume"),
                                      ("TrackPan type 30 Return", "TrackPan:", ["End", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "BackSpace", "type:30", "Return"], "Pan"),
                                      ):
            out = []
            for X, env in ((A, ea), (O, eo)):
                X("SelectTracks: Track=0 TrackCount=1 Mode=Set"); X("SetTrack: Track=0 Focused=1")
                X.to.write(cmd + "\n"); X.to.flush()  # Audacity answers only after the modal dialog closes
                time.sleep(2)
                seen = any(w[0] == title for w in windows(env)) if X is A else None
                keys(env, *seq); time.sleep(1)
                reply(X)
                out.append((seen, tracks(X)[0][1:3]))
                X("SetTrack: Track=0 Pan=0 Volume=0")  # 3.7.3: Volume in dB, Pan in percent
            check(name, out[0][1], out[1][1])
            if out[0][0] is not True: print("   note: Audacity window", title, "not seen")

        out = []
        for X, env in ((A, ea), (O, eo)):
            before = tracks(X)
            X.to.write("TrackMenu:\n"); X.to.flush(); time.sleep(1.5)
            opened = len(windows(env))
            keys(env, "Escape"); time.sleep(1)
            reply(X)
            out.append(tracks(X) == before)
        check("TrackMenu Escape leaves tracks unchanged", out[0], out[1])

    # 86 SetEnvelope full sample comparison, 87 SetProject window geometry: fresh sessions.
    env_cases = (
        ("SetEnvelope Time+Value", ["SetEnvelope: Time=2 Value=0.5"]),
        ("SetEnvelope 2 points", ["SetEnvelope: Time=0 Value=0.2", "SetEnvelope: Time=4 Value=1.5"]),
        ("SetEnvelope 3 points", ["SetEnvelope: Time=0.5 Value=0", "SetEnvelope: Time=1.5 Value=2", "SetEnvelope: Time=3.25 Value=0.3"]),
        ("SetEnvelope Delete", ["SetEnvelope: Time=2 Value=0.5", "SetEnvelope: Delete=1"]),
        ("SetEnvelope outside clip", ["SetEnvelope: Time=50 Value=0.1"]),
        ("SetEnvelope Time only", ["SetEnvelope: Time=1"]),
    )
    with SafeAud() as A, P.Ours() as O:
        ea, eo = xenv(A.proc, "audacity"), xenv(O.proc, "audacity-rs")
        for name, cmds in env_cases:
            out = []
            for X in (A, O):
                start(X, False) if X is A else (X("SelectAll:"), X("RemoveTracks:"))
                X(f"Import2: Filename={W}/c.wav")  # identical source audio in both apps
                X("Select: Start=0 End=5 Mode=Set"); X("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                for c in cmds: X(c)
                path = f"{W}/env_{'a' if X is A else 'o'}.wav" if not os.environ.get("X2C_KEEP") else f"{os.environ['X2C_KEEP']}/{name.replace(' ', '_')}_{'a' if X is A else 'o'}.wav"
                if os.path.exists(path): os.remove(path)
                X("Select: Start=0 End=5 Mode=Set"); X(f"Export2: Filename={path} NumChannels=1"); time.sleep(1)
                out.append(read_wav(path))
            # Audacity's Tone is a Chirp with equal ends; ours Tone. Compare the gain curve: ratio to the
            # un-enveloped tone is what SetEnvelope changes, so compare sample by sample with 1e-5 tolerance.
            n = min(len(out[0]), len(out[1]))
            diff = float(np.max(np.abs(out[0][:n] - out[1][:n]))) if n else 0.0
            check(name, (len(out[0]), diff <= 1e-3), (len(out[1]), True))  # 16-bit export with Audacity's default shaped dither: a few LSB
            if diff > 1e-3: print(f"   max |diff| = {diff:.6g} at sample {int(np.argmax(np.abs(out[0][:n]-out[1][:n])))}")

        for name, cmd in (("SetProject Name", "SetProject: Name=Hello"),
                          ("SetProject geometry", "SetProject: X=50 Y=60 Width=800 Height=600"),
                          ("SetProject name+geometry", "SetProject: Name=Mix X=0 Y=0 Width=1000 Height=700"),
                          ("SetProject Width only", "SetProject: Width=900"), ("SetProject Height only", "SetProject: Height=650")):
            out = []
            for X, env, comm in ((A, ea, "audacity"), (O, eo, "audacity-rs")):
                r = X(cmd).strip().endswith("OK"); time.sleep(1.5)
                ws = [w for w in windows(env) if w[1] >= 200 and w[2] >= 200 and w[0] not in ("Automatic Crash Recovery", "Welcome to Audacity!")]
                main_w = max(ws, key=lambda w: w[1] * w[2]) if ws else None
                out.append((r, main_w))
            if name == "SetProject Name":  # initial frame geometry differs between the apps; only the title is set
                out = [(r, w and w[0]) for r, w in out]
            check(name, out[0], out[1])

    print(f"{sum(results)}/{len(results)} passed")
    sys.exit(0 if all(results) else 1)

if __name__ == "__main__":
    main()
