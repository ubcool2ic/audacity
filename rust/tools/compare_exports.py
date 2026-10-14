#!/usr/bin/env python3
"""Stage 8 export parity: export the same float .aup3 project (with Metadata Editor tags,
written by the Rust port) from Audacity 3.7.3 (Export2 via mod-script-pipe, options set in
audacity.cfg, fresh process per case so the dither's rand() starts from glibc seed 1) and
from the Rust port (fx_tool export). Decodes both with ffmpeg and compares samples
(peak/RMS diff in dBFS, length), compares tags field by field (ffprobe), and reports
whether the files are byte-identical.

Usage: tools/compare_exports.py [workdir] [case-filter]
"""
import os, sys, subprocess, json, re, hashlib
import numpy as np
sys.path.insert(0, os.path.dirname(__file__))
import compare_effects as ce

R = ce.R
W = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else "/tmp/expcmp")
FILT = sys.argv[2] if len(sys.argv) > 2 else ""
os.makedirs(W, exist_ok=True)
TOOL = ce.TOOL
TAGS = {"TITLE": "Parity Ünïcode Title", "ARTIST": "Rust Port", "ALBUM": "Stage 8", "TRACKNUMBER": "3",
        "YEAR": "2026", "GENRE": "Jazz", "COMMENTS": "export comparison", "Custom Field": "custom value"}

# (name, ext, {cfg key under /FileFormats: value}, deterministic-by-design note)
CASES = [
 ("mp3-SET-standard", "mp3", {"MP3RateModeChoice": "SET", "MP3SetRate": 2}),
 ("mp3-SET-insane", "mp3", {"MP3RateModeChoice": "SET", "MP3SetRate": 0}),
 ("mp3-SET-medium", "mp3", {"MP3RateModeChoice": "SET", "MP3SetRate": 3}),
 ("mp3-VBR-q0", "mp3", {"MP3RateModeChoice": "VBR", "MP3VbrRate": 0}),
 ("mp3-VBR-q7", "mp3", {"MP3RateModeChoice": "VBR", "MP3VbrRate": 7}),
 ("mp3-ABR-128", "mp3", {"MP3RateModeChoice": "ABR", "MP3AbrRate": 128}),
 ("mp3-CBR-320", "mp3", {"MP3RateModeChoice": "CBR", "MP3CbrRate": 320}),
 ("mp3-CBR-64", "mp3", {"MP3RateModeChoice": "CBR", "MP3CbrRate": 64}),
 ("ogg-q5", "ogg", {"OggExportQuality": 5}),
 ("ogg-q0", "ogg", {"OggExportQuality": 0}),
 ("ogg-q10", "ogg", {"OggExportQuality": 10}),
 ("opus-default", "opus", {}),
 ("opus-96k-cbr", "opus", {"OPUS/Bitrate": 96000, "OPUS/VbrMode": 0}),
 ("opus-64k-cvbr-speech-10ms", "opus", {"OPUS/Bitrate": 64000, "OPUS/VbrMode": 2, "OPUS/Application": 2048, "OPUS/FrameDuration": 100}),
 ("opus-wideband-q3", "opus", {"OPUS/Cutoff": 1103, "OPUS/Quality": 3}),
 ("wv-normal-16", "wv", {"WavPackEncodeQuality": 1, "WavPackBitDepth": 16}),
 ("wv-high-24", "wv", {"WavPackEncodeQuality": 2, "WavPackBitDepth": 24}),
 ("wv-float32", "wv", {"WavPackEncodeQuality": 3, "WavPackBitDepth": 32}),
 ("wv-hybrid-256", "wv", {"WavPackHybridMode": 1, "WavPackBitrate": 2560, "WavPackBitDepth": 16}),
 ("flac-16-l5", "flac", {"FLACBitDepth": "16", "FLACLevel": "5"}),
 ("flac-24-l8", "flac", {"FLACBitDepth": "24", "FLACLevel": "8"}),
 ("flac-16-l0", "flac", {"FLACBitDepth": "16", "FLACLevel": "0"}),
 ("wav-16", "wav", {}),
 ("wav-24", "wav", {"ExportFormat_SF1_Type/WAV_10000": 3, "Encoding": 3}),
 ("wav-float", "wav", {"ExportFormat_SF1_Type/WAV_10000": 6, "Encoding": 6}),
 ("aiff-16", "aiff", {"ExportFormat_SF1": 131072, "ref": "wav-16"}),
 ("aiff-24", "aiff", {"ExportFormat_SF1": 131072, "Encoding": 3, "ref": "wav-24"}),
]

def set_cfg(opts):
    cfg = os.path.expanduser("~/.config/audacity/audacity.cfg"); s = open(cfg).read()
    # drop previous FileFormats subgroups we manage, then rewrite keys
    groups = {}
    for k, v in opts.items():
        if k == "Encoding": continue
        g, _, key = ("FileFormats/" + k).rpartition("/")
        if key == "OggExportQuality": v = int(v) * 10  # Audacity stores quality x10
        groups.setdefault(g, {})[key] = v
    for sub in ["FileFormats/OPUS", "FileFormats/ExportFormat_SF1_Type"]:
        s = re.sub(r"\[" + re.escape(sub) + r"\]\n(?:[^\[\n][^\n]*\n)*", "", s)
    keys = ["MP3RateModeChoice", "MP3SetRate", "MP3VbrRate", "MP3AbrRate", "MP3CbrRate", "OggExportQuality", "FLACBitDepth", "FLACLevel",
            "WavPackEncodeQuality", "WavPackBitDepth", "WavPackHybridMode", "WavPackCreateCorrectionFile", "WavPackBitrate", "ExportFormat_SF1"]
    s = re.sub(r"(?m)^(" + "|".join(keys) + r")=.*\n", "", s)
    for g, kv in groups.items():
        body = "".join(f"{k}={v}\n" for k, v in kv.items())
        if f"[{g}]\n" in s: s = s.replace(f"[{g}]\n", f"[{g}]\n" + body, 1)
        else: s += f"[{g}]\n" + body
    open(cfg, "w").write(s)

def decode(p):
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", p, "-f", "f32le", "-ac", "2", "-"], capture_output=True, check=True).stdout
    return np.frombuffer(out, dtype="<f4").reshape(-1, 2).T.astype(np.float64)

def tags_of(p):
    j = json.loads(subprocess.run(["ffprobe", "-v", "error", "-show_format", "-show_streams", "-of", "json", p], capture_output=True, text=True).stdout)
    t = {}
    for src in [j.get("format", {}).get("tags", {})] + [s.get("tags", {}) for s in j.get("streams", [])]:
        for k, v in src.items():
            if k.lower() in ("encoder", "vendor_id", "encoded_by"): continue
            t.setdefault(k.upper(), v)
    return t

def main():
    if not os.environ.get("NO_BUILD"): subprocess.run(["cargo", "build", "-q", "--release", "-p", "au-app", "--example", "fx_tool"], cwd=R, check=True)
    sr = 44100; n = sr * 3; t = np.arange(n) / sr; rng = np.random.default_rng(7)
    x = np.stack([0.4 * np.sin(2 * np.pi * 440 * t) + 0.15 * np.sin(2 * np.pi * 3150 * t) + 0.05 * rng.standard_normal(n),
                  0.3 * np.sin(2 * np.pi * 660 * t + 1) + 0.1 * np.sin(2 * np.pi * 7000 * t) + 0.05 * rng.standard_normal(n)]).astype(np.float32)
    ce.write_wav(f"{W}/in.wav", x)
    open(f"{W}/tags.txt", "w").write("".join(f"{k}={v}\n" for k, v in TAGS.items()))
    subprocess.run([TOOL, "mkaup3", f"{W}/in.wav", f"{W}/in.aup3", f"{W}/tags.txt"], check=True)
    n48 = 48000 * 3; t48 = np.arange(n48) / 48000
    x48 = np.stack([0.4 * np.sin(2 * np.pi * 440 * t48) + 0.05 * rng.standard_normal(n48), 0.3 * np.sin(2 * np.pi * 660 * t48) + 0.05 * rng.standard_normal(n48)]).astype(np.float32)
    ce.SR = 48000; ce.write_wav(f"{W}/in48.wav", x48); ce.SR = 44100
    subprocess.run([TOOL, "mkaup3", f"{W}/in48.wav", f"{W}/in48.aup3", f"{W}/tags.txt"], check=True)
    results = []
    for name, ext, opts in CASES:
        if FILT and FILT not in name: continue
        # Export2 picks the format by matching the extension against format *names*,
        # so WavPack must be requested as ".wavpack" (the file content is a normal .wv).
        a, r = f"{W}/{name}.aud.{'wavpack' if ext == 'wv' else ext}", f"{W}/{name}.rs.{ext}"
        # Opus supports 8/12/16/24/48 kHz only; Audacity asks for a rate on 44.1 kHz projects.
        proj = f"{W}/in48.aup3" if ext == "opus" else f"{W}/in.aup3"
        for f in (a, r):
            if os.path.exists(f): os.remove(f)
        opts = dict(opts); ref = opts.pop("ref", None)
        set_cfg(opts)
        err = None
        if ref:
            # Audacity 3.7.3's scripted Export2 cannot produce AIFF on Linux (the typed-PCM
            # options editor drops the header option: it fails, or writes 16-bit WAV). The
            # sample path (mixer + dither + libsndfile) is the same as WAV, so the audio
            # reference is Audacity's WAV export at the same bit depth.
            a = f"{W}/{ref}.aud.wav"
        for _attempt in range(0 if ref else 2):
            try:
                with ce.Aud() as A:
                    A(f"OpenProject2: Filename={proj}")
                    A("SelectAll:")
                    msg = A(f"Export2: Filename={a} NumChannels=2", timeout=180)
                    if "Failed" in msg: print("   ", name, "Export2:", msg.strip()[-300:])
                err = None; break
            except Exception as e:
                err = e
        if err is not None:
            print(name, "AUDACITY FAILED", err); results.append({"name": name, "pass": False, "note": f"audacity: {err}"}); continue
        kv = [f"{k}={v}" for k, v in opts.items()]
        p = subprocess.run([TOOL, "export", proj, r] + kv, capture_output=True, text=True)
        if p.returncode != 0 or not os.path.exists(a):
            print(name, "FAILED", p.stderr[-400:], os.path.exists(a)); results.append({"name": name, "pass": False, "note": p.stderr[-200:]}); continue
        ident = hashlib.sha1(open(a, "rb").read()).digest() == hashlib.sha1(open(r, "rb").read()).digest()
        da, dr = decode(a), decode(r)
        m = min(da.shape[1], dr.shape[1]); d = da[:, :m] - dr[:, :m]
        peak = ce.db(np.abs(d).max() if m else 1); rms = ce.db(np.sqrt((d ** 2).mean()) if m else 1)
        ta, tr = tags_of(a), tags_of(r)
        if ref:  # AIFF stores ARTIST as the AUTH chunk (libsndfile), which WAV has no field for
            tr.pop("AUTHOR", None)
        tag_diff = {k: (ta.get(k), tr.get(k)) for k in set(ta) | set(tr) if ta.get(k) != tr.get(k)}
        ok = da.shape[1] == dr.shape[1] and rms <= -120 and not tag_diff
        res = {"name": name, "ext": ext, "opts": opts, "len": [da.shape[1], dr.shape[1]], "peak_db": round(peak, 1), "rms_db": round(rms, 1),
               "byte_identical": bool(ident), "tags_aud": ta, "tag_diff": tag_diff, "size": [os.path.getsize(a), os.path.getsize(r)], "pass": bool(ok)}
        results.append(res)
        print(f"{'PASS' if ok else 'FAIL'} {name:28s} len {da.shape[1]}/{dr.shape[1]} rms {rms:7.1f} peak {peak:7.1f} bytes={'same' if ident else 'diff'} size {res['size']} tags {'ok' if not tag_diff else tag_diff}")
    with open(f"{W}/results.jsonl", "w") as f:
        for r_ in results: f.write(json.dumps(r_, ensure_ascii=False) + "\n")
    print(f"{sum(r_['pass'] for r_ in results)}/{len(results)} pass")

if __name__ == "__main__":
    main()
