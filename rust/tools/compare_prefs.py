#!/usr/bin/env python3
"""Preferences behaviour vs Audacity 3.7.3 over mod-script-pipe (both apps driven by the same
SetPreference commands): default track names, effects-menu grouping, Quality default sample format."""
import json, os, re, subprocess, sys, tempfile, time, select
sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as C
R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
W = C.W

class Ours:
    def __enter__(self):
        self.dir = tempfile.mkdtemp(prefix="au-prefs-")
        exe = f"{R}/target/debug/audacity-rs"
        env = dict(os.environ, AU_SCRIPT_PIPE="1", AU_SCRIPT_PIPE_DIR=self.dir, AU_ALLOW_TEST_HOOKS="1",
                   AU_PREFS_PATH=f"{self.dir}/audacity.cfg", AU_TOOLBARS_CFG=f"{self.dir}/tb.cfg", AU_THEME_DIR=f"{self.dir}/theme",
                   AUDACITY_RS_CRASH_DIR=f"{self.dir}/crash")
        self.proc = subprocess.Popen(["xvfb-run", "-a", exe], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        to, fr = f"{self.dir}/audacity_script_pipe.to", f"{self.dir}/audacity_script_pipe.from"
        for _ in range(200):
            if os.path.exists(to) and os.path.exists(fr): break
            time.sleep(0.1)
        self.to = open(to, "w"); self.fr = os.open(fr, os.O_RDONLY); self.buf = b""; return self
    def __call__(self, c, timeout=60):
        self.to.write(c + "\n"); self.to.flush(); end = time.time() + timeout
        while b"\n\n" not in self.buf:
            if not select.select([self.fr], [], [], max(0, end - time.time()))[0]: raise TimeoutError(c)
            self.buf += os.read(self.fr, 65536)
        r, self.buf = self.buf.split(b"\n\n", 1); return r.decode(errors="replace")
    def __exit__(self, *a):
        import signal
        try: os.killpg(self.proc.pid, signal.SIGTERM)
        except ProcessLookupError: pass
        self.proc.wait(timeout=10)

def body(r): return "\n".join(l for l in r.splitlines() if not l.startswith("BatchCommand finished"))
def jload(r):
    s = body(r); s = s[s.index("["):s.rindex("]") + 1]
    return json.loads(re.sub(r'\\(?![\\"/bfnrtu])', r'\\\\', s))

def aud_effect_menu(A, top="Effect"):
    m = jload(A("GetInfo: Type=Menus Format=JSON"))
    out, inside = [], False
    for e in m:
        if e["depth"] == 0:
            inside = e["label"] == top; continue
        if inside: out.append((e["depth"] - 1, e["label"].replace("...", "").replace("…", "").strip()))
    return out

def ours_effect_menu(O, kind="Effect"):
    lines = body(O(f"GetEffectMenu: Kind={kind}")).splitlines()
    return [(int(d), l.replace("…", "").strip()) for d, l in (x.split("\t", 1) for x in lines if "\t" in x)]

def common_order(a, b):
    # Names Audacity lists more than once come from system LADSPA plug-ins
    # (e.g. /usr/lib/ladspa Vocoder) that we do not load: ignore them.
    from collections import Counter
    dup = {l for l, n in Counter(l for _, l in a).items() if n > 1}
    a = [(d, l) for d, l in a if l not in dup]; b = [(d, l) for d, l in b if l not in dup]
    names_b = {l for _, l in b}; names_a = {l for _, l in a}
    sa = [(d, l) for d, l in a if l in names_b and l != "----"]; sb = [(d, l) for d, l in b if l in names_a and l != "----"]
    return sa == sb, sa, sb

def main():
    subprocess.run(["cargo", "build", "-q", "-p", "au-app", "--features", "gui"], cwd=R, check=True)
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "au-app", "--example", "fx_tool"], cwd=R, check=True)
    C.make_inputs()
    res = []
    def report(name, ok, detail):
        res.append((name, ok, detail)); print(f"{'PASS' if ok else 'FAIL'} {name:40s} {detail}", flush=True)
    with C.Aud() as A:
      for section in (1, 2, 3):
       with Ours() as O:
        if section != 1: pass
        # 1. Default audio track name (Tracks) + MakeUniqueTrackName
        for name in (["", "Foo"] if section == 1 else []):
            for X in (A, O):
                X("SelectAll:"); X("RemoveTracks:") if X is A else None
                X(f'SetPreference: Name=/GUI/TrackNames/DefaultTrackName Value="{name or "Audio"}" Reload=1')
            if O("GetInfo: Type=Tracks Format=JSON").count('"name"'):  # ours: fresh app per case
                pass
            for X in (A, O):
                X("NewMonoTrack:"); X("NewStereoTrack:")
            ta = [t["name"] for t in jload(A("GetInfo: Type=Tracks Format=JSON"))]
            to = [t["name"] for t in jload(O("GetInfo: Type=Tracks Format=JSON"))]
            to = to[-2:]; ta = ta[-2:]
            report(f"track names default={name or '(Audio)'}", ta == to, f"aud {ta} ours {to}")
        # 2. Effects menu grouping
        for g in [] if section != 2 else ["sortby:name", "sortby:publisher:name", "sortby:type:name", "groupby:publisher", "groupby:type", "default"]:
            for X in (A, O): X(f'SetPreference: Name=/Effects/GroupBy Value="{g}" Reload=1')
            time.sleep(1)
            for top, kind in [("Effect", "Effect"), ("Generate", "Generate"), ("Analyze", "Analyze")]:
                ok, sa, sb = common_order(aud_effect_menu(A, top), ours_effect_menu(O, kind))
                report(f"menu {top} {g}", ok, f"{len(sa)} common items" + ("" if ok else f"\n   aud  {sa}\n   ours {sb}"))
        # 3. Quality > Default Sample Format = 16-bit: imported float audio is quantized (dither None)
        for fmt in ([] if section != 3 else ["Format16Bit", "Format24Bit"]):
            out = {}
            for tag, X in (("aud", A), ("rs", O)):
                X(f"SetPreference: Name=/SamplingRate/DefaultProjectSampleFormatChoice Value={fmt}")
                X("SetPreference: Name=/Quality/DitherAlgorithmChoice Value=None")
                X("SelectAll:"); X("RemoveTracks:") if X is A else None
                X(f"Import2: Filename={W}/in.wav")
                ap = f"{W}/q_{fmt}.{tag}.aup3"
                if os.path.exists(ap): os.remove(ap)
                X(f"SaveProject2: Filename={ap}"); time.sleep(1)
                subprocess.run([f"{R}/target/release/examples/fx_tool", "aup3wav", ap, f"{W}/q_{fmt}.{tag}.wav"], check=True, capture_output=True)
                out[tag] = C.read_wav(f"{W}/q_{fmt}.{tag}.wav")
            a, b = out["aud"], out["rs"]
            import numpy as np
            n = min(a.shape[1], b.shape[1]); d = float(np.max(np.abs(a[:, :n] - b[:, :n]))) if n and a.shape[0] == b.shape[0] else float("nan")
            report(f"default format {fmt} import", a.shape == b.shape and d == 0.0, f"max diff {d:g} shapes {a.shape} {b.shape}")
        A("SetPreference: Name=/SamplingRate/DefaultProjectSampleFormatChoice Value=Format32BitFloat")
    # 4. Tracks Behaviors editing: EditClipsCanMove, PasteAsNewClips (fresh app of ours per case)
    with C.Aud() as A:
      def clips(X):
          return sorted((round(c["start"], 4), round(c["end"], 4)) for c in jload(X("GetInfo: Type=Clips Format=JSON")))
      cases = [("move=1", {"/GUI/EditClipCanMove": 1, "/GUI/PasteAsNewClips": 0}),
               ("move=0", {"/GUI/EditClipCanMove": 0, "/GUI/PasteAsNewClips": 0}),
               ("newclips=1", {"/GUI/EditClipCanMove": 1, "/GUI/PasteAsNewClips": 1})]
      for name, prefs in cases:
        with Ours() as O:
          got = {}
          for tag, X in (("aud", A), ("rs", O)):
              for k, v in prefs.items(): X(f"SetPreference: Name={k} Value={v} Reload=1")
              if X is A: X("SelectAll:"); X("RemoveTracks:")
              X(f"Import2: Filename={W}/in.wav")
              steps = []
              for cmd in ["Select: Start=0.8 End=1.0 Track=0", "SplitDelete:", "Select: Start=0.2 End=0.4 Track=0", "Delete:"]:
                  X(cmd)
              steps.append(clips(X))
              X("Select: Start=0.05 End=0.55 Track=0"); X("Copy:")
              X("Select: Start=0.3 End=0.3 Track=0"); X("Paste:")
              steps.append(clips(X))
              got[tag] = steps
          report(f"editing {name}", got["aud"] == got["rs"], f"aud {got['aud']} ours {got['rs']}")

    # 5. Quality > Default Sample Format: generated audio in a new 16/24-bit track is stored quantized
    import numpy as np
    cfgp = os.path.expanduser("~/.config/audacity/audacity.cfg")
    for fmt, hq in (("Format16Bit", "None"), ("Format24Bit", "None"), ("Format16Bit", "Shaped")):
     # Audacity caches Dither::BestSetting; set it in its cfg before launching.
     c = open(cfgp).read(); c = re.sub(r"HQDitherAlgorithmChoice=.*\n", "", c)
     c = c.replace("[Quality]\n", f"[Quality]\nHQDitherAlgorithmChoice={hq}\n", 1) if "[Quality]\n" in c else c + f"\n[Quality]\nHQDitherAlgorithmChoice={hq}\n"; open(cfgp, "w").write(c)
     with C.Aud() as A:
        with Ours() as O:
          out = {}
          for tag, X in (("aud", A), ("rs", O)):
              X(f"SetPreference: Name=/SamplingRate/DefaultProjectSampleFormatChoice Value={fmt}")
              if X is O: X(f"SetPreference: Name=/Quality/HQDitherAlgorithmChoice Value={hq}")
              if X is A: X("SelectAll:"); X("RemoveTracks:")
              X("NewMonoTrack:"); X("Select: Start=0 End=1 Track=0")
              X("Chirp: StartFreq=441 EndFreq=441 StartAmp=0.5 EndAmp=0.5 Waveform=Sine Interpolation=Linear")  # Tone via pipe skips PostSet in 3.7.3; Chirp == Tone dialog
              ap = f"{W}/gen_{fmt}.{tag}.aup3"
              if os.path.exists(ap): os.remove(ap)
              X(f"SaveProject2: Filename={ap}"); time.sleep(1)
              for _ in range(40):  # wait for Audacity to checkpoint the WAL
                  if not os.path.exists(ap + "-wal"): break
                  time.sleep(0.25)
              subprocess.run([f"{R}/target/release/examples/fx_tool", "aup3wav", ap, f"{W}/gen_{fmt}.{tag}.wav"], check=True, capture_output=True)
              out[tag] = C.read_wav(f"{W}/gen_{fmt}.{tag}.wav")
          a, b = out["aud"], out["rs"]
          scale = 32768.0 if fmt == "Format16Bit" else 8388608.0
          on_grid = float(np.max(np.abs(b * scale - np.round(b * scale)))) if b.size else 1.0
          n = min(a.shape[1], b.shape[1]); d = float(np.max(np.abs(a[:, :n] - b[:, :n]))) * scale if n else float("nan")
          if hq != "None":  # random dither: compare noise level, not samples
              ideal = 0.5 * np.sin(2 * np.pi * 441 * np.arange(a.shape[1]) / 44100)
              ea = float(np.std(a[0] - ideal)) * scale; eb = float(np.std(b[0] - ideal)) * scale
              report(f"generate into {fmt} track, HQ dither {hq}", a.shape == b.shape and on_grid < 1e-3 and abs(ea - eb) < 0.3 * ea,
                     f"error vs ideal sine: aud {ea:.2f} LSB rms, ours {eb:.2f} LSB rms"); continue
          report(f"generate into {fmt} track", a.shape == b.shape and on_grid < 1e-3 and d <= 1.0,
                 f"ours off-grid {on_grid:g}, max diff {d:g} LSB, shapes {a.shape} {b.shape}")
        A("SetPreference: Name=/SamplingRate/DefaultProjectSampleFormatChoice Value=Format32BitFloat")

    # 6. Interface > Show 'Extra' menus: the Extra top-level menu appears exactly when set.
    for on in (0, 1):
        c = open(cfgp).read(); c = re.sub(r"ShowExtraMenus=.*\n", "", c)
        c = c.replace("[GUI]\n", f"[GUI]\nShowExtraMenus={on}\n", 1) if "[GUI]\n" in c else c + f"\n[GUI]\nShowExtraMenus={on}\n"; open(cfgp, "w").write(c)
        with C.Aud() as A:
            aud = [m["label"].replace("&", "") for m in jload(A("GetInfo: Type=Menus Format=JSON")) if m["depth"] == 0]
        with Ours() as O:
            O(f"SetPreference: Name=/GUI/ShowExtraMenus Value={on}"); time.sleep(0.5); O("GetPreference: Name=/GUI/ShowExtraMenus")
            ours = body(O("GetMenuTitles:")).split("\t")
        report(f"Extra menu shown={on}", ("Extra" in aud) == ("Extra" in ours) == bool(on), f"aud {'Extra' in aud} ours {'Extra' in ours}")

    print(f"{sum(1 for r in res if r[1])}/{len(res)} pass")

if __name__ == "__main__":
    # The run changes Audacity's own preferences; put the user's file back afterwards.
    cfg = os.path.expanduser("~/.config/audacity/audacity.cfg"); saved = open(cfg).read()
    try:
        main()
    finally:
        open(cfg, "w").write(saved)
