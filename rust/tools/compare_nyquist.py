#!/usr/bin/env python3
"""Stage 9 Nyquist parity: runs Audacity 3.7.3's shipped .ny plug-ins in real Audacity
(scripted) and through au-nyquist (ny_run example, Audacity's own interpreter library
loaded at runtime), then compares samples / labels.
Usage: tools/compare_nyquist.py [workdir] [filter]"""
import os, sys, subprocess, json
sys.argv = sys.argv[:1] + [sys.argv[1] if len(sys.argv) > 1 else "/tmp/nycmp"] + sys.argv[2:]
sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
import numpy as np
W, R = C.W, C.R
P = "/usr/share/audacity/plug-ins/"
NY = os.path.join(R, "target/release/examples/ny_run")
FILT = sys.argv[2] if len(sys.argv) > 2 else ""
# name, audacity cmd id, file, params, kind (fx/gen/lab/spec), spectral (f0,f1) or None
CASES = [
 ("Low-pass", "Low-passFilter", "lowpass.ny", {"FREQUENCY": 1000, "ROLLOFF": "dB24"}, "fx", None),
 ("High-pass", "High-passFilter", "highpass.ny", {"FREQUENCY": 1000, "ROLLOFF": "dB12"}, "fx", None),
 ("Notch", "NotchFilter", "notch.ny", {"FREQUENCY": 220, "Q": 1}, "fx", None),
 ("Shelf", "ShelfFilter", "ShelfFilter.ny", {"TYPE": "Low", "HZ": 500, "GAIN": 6}, "fx", None),
 ("Tremolo", "Tremolo", "tremolo.ny", {"WAVE": "Sine", "PHASE": 0, "WET": 40, "LFO": 4}, "fx", None),
 ("Delay", "Delay", "delay.ny", {"DELAY-TYPE": "Regular", "DGAIN": -6, "DELAY": 0.3, "PITCH-TYPE": "PitchTempo", "SHIFT": 0, "NUMBER": 3, "CONSTRAIN": "Yes"}, "fx", None),
 ("NoiseGate", "NoiseGate", "noisegate.ny", {"MODE": "Gate", "STEREO-LINK": "LinkStereo", "THRESHOLD": -30, "GATE-FREQ": 0, "LEVEL-REDUCTION": -24, "ATTACK": 10, "HOLD": 50, "DECAY": 100}, "fx", None),
 ("ClipFix", "ClipFix", "clipfix.ny", {"THRESHOLD": 95, "GAIN": -9}, "fx", None),
 ("StudioFadeOut", "StudioFadeOut", "StudioFadeOut.ny", {}, "fx", None),
 ("AdjustableFade", "AdjustableFade", "adjustable-fade.ny", {"TYPE": "Up", "CURVE": 20, "UNITS": "Percent", "GAIN0": 0, "GAIN1": 100, "PRESET": "None"}, "fx", None),
 ("LegacyLimiter", "LegacyLimiter", "legacy-limiter.ny", {"TYPE": "SoftLimit", "GAIN-L": 0, "GAIN-R": 0, "THRESH": -6, "HOLD": 10, "MAKEUP": "No"}, "fx", None),
 ("RissetDrum", "RissetDrum", "rissetdrum.ny", {"FREQ": 100, "DECAY": 2, "CF": 500, "BW": 400, "NOISE": 0, "GAIN": 0.8}, "gen", None),
 ("Pluck", "Pluck", "pluck.ny", {"PITCH": 60, "FADE": "Gradual", "DUR": 1}, "gen", None),
 ("LabelSounds", "LabelSounds", "label-sounds.ny", {"THRESHOLD": -30, "MEASUREMENT": "peak", "SIL-DUR": 0.1, "SND-DUR": 0.1, "TYPE": "region", "PRE-OFFSET": 0, "POST-OFFSET": 0, "TEXT": "Sound ##1"}, "lab", None),
 ("BeatFinder", "BeatFinder", "beat.ny", {"THRESVAL": 65}, "lab", None),
 ("SpectralDelete", "SpectralDelete", "spectral-delete.ny", {}, "fx", (1000.0, 4000.0)),
 ("SpectralMulti", "SpectralEditMultiTool", "SpectralEditMulti.ny", {}, "fx", (500.0, 2000.0)),
 ("SpectralPEQ", "SpectralEditParametricEq", "SpectralEditParametricEQ.ny", {"CONTROL-GAIN": 6}, "fx", (200.0, 800.0)),
 ("SpectralShelves", "SpectralEditShelves", "SpectralEditShelves.ny", {"CONTROL-GAIN": -6}, "fx", (300.0, 3000.0)),
]

def main():
    C.make_inputs()
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "au-app", "--example", "fx_tool"], cwd=R, check=True)
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "au-nyquist", "--example", "ny_run"], cwd=R, check=True)
    cases = [c for c in CASES if not FILT or c[0] in FILT.split(",")]
    res = []
    with C.Aud() as A:
        for name, cid, f, params, kind, spec in cases:
            row = {"case": name}
            try:
                A("SelectAll:"); A("RemoveTracks:")
                src = f"{W}/beats.wav" if name == "BeatFinder" else f"{W}/in.wav"
                if kind == "gen":
                    A("NewStereoTrack:"); A("SelectTime: Start=0 End=2 RelativeTo=ProjectStart")
                else:
                    A(f"Import2: Filename={src}"); A("SelectAll:")
                extra = []
                if spec:
                    A('SetTrackVisuals: Display="Spectrogram"')  # spectral effects need the spectrogram view
                    A(f"SelectFrequencies: Low={spec[0]} High={spec[1]}")
                    extra = [f"__f0={spec[0]}", f"__f1={spec[1]}"]
                A(cid + ":" + "".join(f' {k}="{v}"' for k, v in params.items()))
                rp = [f"{k}={v}" for k, v in params.items()] + extra
                if kind == "lab":
                    info = A("GetInfo: Type=Labels Format=JSON"); al = C.labels_from(info)
                    out = subprocess.run([NY, P + f, src, "-", "0", "2"] + rp, capture_output=True, text=True, check=True).stdout
                    rl = [(round(float(a), 3), round(float(b), 3), c) for _, a, b, c in (l.split("\t") for l in out.splitlines() if l.startswith("LABEL"))]
                    ok = len(al) == len(rl) and all(abs(x[0]-y[0]) <= 1e-3 and abs(x[1]-y[1]) <= 1e-3 and x[2] == y[2] for x, y in zip(rl, al))
                    row.update(aud=len(al), rust=len(rl), pass_=ok, detail=f"{al[:4]} | {rl[:4]}")
                else:
                    ap = f"{W}/{name}.aud.aup3"
                    if os.path.exists(ap): os.remove(ap)
                    A(f"SaveProject2: Filename={ap}")
                    C.rust(["aup3wav", ap, f"{W}/{name}.aud.wav"]); a = C.read_wav(f"{W}/{name}.aud.wav")
                    if kind == "gen":
                        C.write_wav(f"{W}/zero.wav", np.zeros((2, 2 * C.SR)))
                        srcw = f"{W}/zero.wav"
                    else: srcw = src
                    subprocess.run([NY, P + f, srcw, f"{W}/{name}.rs.wav", "0", "2"] + rp, check=True, capture_output=True, text=True)
                    b = C.read_wav(f"{W}/{name}.rs.wav")
                    n = min(a.shape[1], b.shape[1]); ch = min(a.shape[0], b.shape[0]); d = a[:ch, :n] - b[:ch, :n]
                    rms = C.db(np.sqrt(np.mean(d**2))) if n else 0
                    ok = a.shape == b.shape and rms <= -120
                    row.update(len_aud=list(a.shape), len_rust=list(b.shape), rms_diff_db=round(rms, 1), peak_diff_db=round(C.db(np.max(np.abs(d))), 1) if n else 0, pass_=bool(ok))
            except Exception as e:
                row.update(pass_=False, error=str(e)[-400:])
            res.append(row); print(json.dumps(row), flush=True)
    json.dump(res, open(f"{W}/ny_results.json", "w"), indent=1)
    print(f"PASS {sum(r['pass_'] for r in res)}/{len(res)}")

if __name__ == "__main__":
    main()
