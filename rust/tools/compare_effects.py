#!/usr/bin/env python3
"""Stage 5/6 parity check: runs each effect/generator/analyzer in real Audacity 3.x
(headless xvfb + mod-script-pipe) and in the Rust port (fx_tool), then compares.

Usage: tools/compare_effects.py [workdir] [case-filter]
Audacity output is read from a float32 .aup3 saved by Audacity (exact samples, no
export quantization) via `fx_tool aup3wav`. Metrics: max |diff| (peak err) and RMS of
the difference, in dBFS, plus length match. Analyzers compare label sets.
"""
import os, sys, subprocess, json, time, re, wave, struct
import numpy as np

R = os.path.realpath(os.path.join(os.path.dirname(__file__), ".."))
W = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else "/tmp/fxcmp")
FILT = sys.argv[2] if len(sys.argv) > 2 else ""
os.makedirs(W, exist_ok=True)
TOOL = os.path.join(R, "target/release/examples/fx_tool")
SR = 44100

# (name, audacity command id, params {Key: value} (same keys for both apps; Rust matches
#  Nyquist keys case-insensitively), rust id, kind, pass threshold for RMS diff in dBFS
#  (0 = stochastic: length + level within 1 dB), note)
# kind: fx = process in.wav, aux = needs control track (2nd track), gen = generate 2 s stereo,
#       gennoise = random generator, lab = analyzer (compare labels)
CASES = [
 ("Amplify", "Amplify", {"Ratio": 0.5}, "Amplify", "fx", -120, "exact"),
 ("Normalize", "Normalize", {"PeakLevel": -3, "ApplyVolume": 1, "RemoveDcOffset": 1, "StereoIndependent": 0}, "Normalize", "fx", -120, "exact"),
 ("Invert", "Invert", {}, "Invert", "fx", -120, "exact"),
 ("Reverse", "Reverse", {}, "Reverse", "fx", -120, "exact"),
 ("FadeIn", "FadeIn", {}, "FadeIn", "fx", -110, "exact"),
 ("FadeOut", "FadeOut", {}, "FadeOut", "fx", -110, "exact"),
 ("Repeat", "Repeat", {"Count": 2}, "Repeat", "fx", -120, "exact"),
 ("BassAndTreble", "BassAndTreble", {"Bass": 6, "Treble": -4, "Gain": 0}, "BassAndTreble", "fx", -100, "biquad"),
 ("ClassicFilters", "ClassicFilters", {"FilterType": "Butterworth", "FilterSubtype": "Lowpass", "Order": 4, "Cutoff": 1000}, "ClassicFilters", "ref_classic", -100, "ref: Audacity lib-math.so (3.7.3 crashes when scripted)"),
 ("Cheby", "ClassicFilters", {"FilterType": "Chebyshev Type I", "FilterSubtype": "Highpass", "Order": 3, "Cutoff": 2000, "PassbandRipple": 1}, "ClassicFilters", "ref_classic", -100, "ref: Audacity lib-math.so (3.7.3 crashes when scripted)"),
 ("High-pass", "High-passFilter", {"FREQUENCY": 1000, "ROLLOFF": "dB12"}, "High-passFilter", "fx", -80, "nyquist"),
 ("Low-pass", "Low-passFilter", {"FREQUENCY": 1000, "ROLLOFF": "dB24"}, "Low-passFilter", "fx", -80, "nyquist"),
 ("Notch", "NotchFilter", {"FREQUENCY": 220, "Q": 1}, "NotchFilter", "fx", -80, "nyquist"),
 ("Shelf", "ShelfFilter", {"TYPE": "Low", "HZ": 500, "GAIN": 6}, "ShelfFilter", "fx", -80, "nyquist"),
 ("Echo", "Echo", {"Delay": 0.25, "Decay": 0.5}, "Echo", "fx", -120, "exact"),
 ("Phaser", "Phaser", {"Stages": 4, "DryWet": 128, "Freq": 0.4, "Phase": 0, "Depth": 100, "Feedback": 20, "Gain": -6}, "Phaser", "fx", -100, ""),
 ("Wahwah", "Wahwah", {"Freq": 1.5, "Phase": 0, "Depth": 70, "Resonance": 2.5, "Offset": 30, "Gain": -6}, "Wahwah", "fx", -100, ""),
 ("Distortion", "Distortion", {"Type": "Hard Clipping", "DC Block": 0, "Threshold dB": -12, "Noise Floor": -70, "Parameter 1": 50, "Parameter 2": 50, "Repeats": 1}, "Distortion", "fx", -100, ""),
 ("DistortionCubic", "Distortion", {"Type": "Cubic Curve (odd harmonics)", "DC Block": 0, "Threshold dB": -6, "Noise Floor": -70, "Parameter 1": 50, "Parameter 2": 50, "Repeats": 2}, "Distortion", "fx", -100, ""),
 ("DistortionLeveller", "Distortion", {"Type": "Leveller", "DC Block": 1, "Threshold dB": -6, "Noise Floor": -70, "Parameter 1": 0, "Parameter 2": 50, "Repeats": 3}, "Distortion", "fx", -100, ""),
 ("Tremolo", "Tremolo", {"WAVE": "Sine", "PHASE": 0, "WET": 40, "LFO": 4}, "Tremolo", "fx", -80, "nyquist"),
 ("Delay", "Delay", {"DELAY-TYPE": "Regular", "DGAIN": -6, "DELAY": 0.3, "PITCH-TYPE": "PitchTempo", "SHIFT": 0, "NUMBER": 3, "CONSTRAIN": "Yes"}, "Delay", "fx", -80, "nyquist"),
 ("Reverb", "Reverb", {"RoomSize": 75, "Delay": 10, "Reverberance": 50, "HfDamping": 50, "ToneLow": 100, "ToneHigh": 100, "WetGain": -1, "DryGain": -1, "StereoWidth": 100, "WetOnly": 0}, "Reverb", "fx", -100, ""),
 ("Compressor", "Compressor", {"thresholdDb": -20, "makeupGainDb": 0, "kneeWidthDb": 5, "compressionRatio": 4, "lookaheadMs": 1, "attackMs": 30, "releaseMs": 150}, "Compressor", "fx", -90, ""),
 ("Limiter", "Limiter", {"thresholdDb": -10, "makeupTargetDb": -1, "kneeWidthDb": 2, "lookaheadMs": 1, "releaseMs": 20}, "Limiter", "fx", -90, ""),
 ("LegacyCompressor", "LegacyCompressor", {"Threshold": -20, "NoiseFloor": -40, "Ratio": 3, "AttackTime": 0.2, "ReleaseTime": 1, "Normalize": 0, "UsePeak": 0}, "LegacyCompressor", "fx", -90, ""),
 ("NoiseGate", "NoiseGate", {"MODE": "Gate", "STEREO-LINK": "LinkStereo", "THRESHOLD": -30, "GATE-FREQ": 0, "LEVEL-REDUCTION": -24, "ATTACK": 10, "HOLD": 50, "DECAY": 100}, "NoiseGate", "fx", -70, "nyquist"),
 ("AutoDuck", "AutoDuck", {"DuckAmountDb": -12, "ThresholdDb": -30, "MaximumPause": 1}, "AutoDuck", "aux", -100, ""),
 ("GraphicEq", "GraphicEq", {"FilterLength": 8191}, "GraphicEq", "fx", -100, "flat curve"),
 ("LoudnessNormalization", "LoudnessNormalization", {"StereoIndependent": 0, "LUFSLevel": -23, "RMSLevel": -20, "DualMono": 1, "NormalizeTo": 0}, "LoudnessNormalization", "fx", -100, ""),
 ("LoudnessRMS", "LoudnessNormalization", {"StereoIndependent": 0, "LUFSLevel": -23, "RMSLevel": -20, "DualMono": 1, "NormalizeTo": 1}, "LoudnessNormalization", "fx", -100, ""),
 ("ClickRemoval", "ClickRemoval", {"Threshold": 200, "Width": 20}, "ClickRemoval", "fx", -100, ""),
 ("ChangeSpeed", "ChangeSpeedAndPitch", {"Percentage": 25}, "ChangeSpeed", "fx", -120, "system libsoxr 0.1.3 (same as Audacity)"),
 ("ChangeTempo", "ChangeTempo", {"Percentage": 20, "SBSMS": 0}, "ChangeTempo", "fx", -120, "system libSoundTouch 2.4.0 (same as Audacity)"),
 ("ChangePitch", "ChangePitch", {"Percentage": 20, "SBSMS": 0}, "ChangePitch", "fx", -120, "system libSoundTouch 2.4.0 (same as Audacity)"),
 ("Paulstretch", "Paulstretch", {"Stretch Factor": 3, "Time Resolution": 0.25}, "Paulstretch", "fx", -120, "glibc rand() phases"),
 ("TruncateSilence", "TruncateSilence", {"Threshold": -40, "Action": "Truncate Detected Silence", "Minimum": 0.2, "Truncate": 0.1, "Compress": 50, "Independent": 0}, "TruncateSilence", "fx", -100, ""),
 ("StereoToMono", "StereoToMono", {}, "StereoToMono", "fx", -120, ""),
 ("AdjustableFade", "AdjustableFade", {"TYPE": "Up", "CURVE": 0, "UNITS": "Percent", "GAIN0": 0, "GAIN1": 100, "PRESET": "None"}, "AdjustableFade", "fx", -80, "nyquist"),
 ("StudioFadeOut", "StudioFadeOut", {}, "StudioFadeOut", "fx", -60, "nyquist"),
 ("ClipFix", "ClipFix", {"THRESHOLD": 80, "GAIN": -6}, "ClipFix", "fx", -60, "nyquist"),
 ("Tone", "Tone", {"Frequency": 440, "Amplitude": 0.8, "Waveform": "Sine"}, "Tone", "gen", -120, ""),
 ("ToneSquareNA", "Tone", {"Frequency": 1000, "Amplitude": 0.5, "Waveform": "Square, no alias"}, "Tone", "gen", -100, ""),
 ("ToneSaw", "Tone", {"Frequency": 300, "Amplitude": 0.5, "Waveform": "Sawtooth"}, "Tone", "gen", -100, ""),
 ("Chirp", "Chirp", {"StartFreq": 100, "EndFreq": 4000, "StartAmp": 0.8, "EndAmp": 0.1, "Waveform": "Sine", "Interpolation": "Logarithmic"}, "Chirp", "gen", -100, ""),
 ("DTMF", "DtmfTones", {"Sequence": "audacity", "Duty Cycle": 55, "Amplitude": 0.8}, "DtmfTones", "gen", -100, ""),
 ("NoiseWhite", "Noise", {"Type": "White", "Amplitude": 0.5}, "Noise", "gennoise", 0, "random; level only"),
 ("NoisePink", "Noise", {"Type": "Pink", "Amplitude": 0.5}, "Noise", "gennoise", 0, "random; level only"),
 ("NoiseBrown", "Noise", {"Type": "Brownian", "Amplitude": 0.5}, "Noise", "gennoise", 0, "random; level only"),
 ("Silence", "Silence", {}, "Silence", "gen", -120, ""),
 ("RhythmTrack", "RhythmTrack", {"TEMPO": 120, "TIMESIG": 4, "SWING": 0, "BARS": 1, "CLICK-TRACK-DUR": 0, "OFFSET": 0, "CLICK-TYPE": "Metronome", "HIGH": 84, "LOW": 80}, "RhythmTrack", "gen", -40, "nyquist"),
 ("Pluck", "Pluck", {"PITCH": 60, "FADE": "Abrupt", "DUR": 1}, "Pluck", "gen", -120, "nyquist (krand, deterministic)"),
 ("FindClipping", "FindClipping", {"Duty Cycle Start": 3, "Duty Cycle End": 3}, "FindClipping", "lab", 0, ""),
 ("BeatFinder", "BeatFinder", {"THRESVAL": 60}, "BeatFinder", "lab", 0, ""),
 ("LabelSounds", "LabelSounds", {"THRESHOLD": -30, "MEASUREMENT": "peak", "SIL-DUR": 0.1, "SND-DUR": 0.1, "TYPE": "region", "PRE-OFFSET": 0, "POST-OFFSET": 0, "TEXT": "S"}, "LabelSounds", "lab", 0, ""),
 ("RissetDrum", "RissetDrum", {"FREQ": 100, "DECAY": 2, "CF": 500, "BW": 400, "NOISE": 25, "GAIN": 0.8}, "RissetDrum", "genfresh", -100, "nyquist; glibc rand() noise (fresh Audacity)"),
 ("SampleDataExport", "SampleDataExport", {"NUMBER": 200, "UNITS": "Linear", "FILEFORMAT": "Count", "HEADER": "None", "OPTEXT": "", "CHANNEL-LAYOUT": "SameLine", "MESSAGES": "None"}, "SampleDataExport", "txt", 0, "nyquist; exported text compared byte-for-byte"),
 ("SampleDataExportDB", "SampleDataExport", {"NUMBER": 500, "UNITS": "dB", "FILEFORMAT": "Time", "HEADER": "Standard", "OPTEXT": "hello", "CHANNEL-LAYOUT": "Alternate", "MESSAGES": "None"}, "SampleDataExport", "txt", 0, "nyquist; exported text compared byte-for-byte"),
 ("PlotSpectrum_0_3_1024", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Hann window", "Size": 1024, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_0_128", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Rectangular window", "Size": 128, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_5_65536", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Blackman-Harris window", "Size": 65536, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_9_4096", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Gaussian(a=4.5) window", "Size": 4096, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_2_2048", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Hamming window", "Size": 2048, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_1_512", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Bartlett window", "Size": 512, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_4_8192", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Blackman window", "Size": 8192, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_6_256", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Welch window", "Size": 256, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_7_16384", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Gaussian(a=2.5) window", "Size": 16384, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_0_8_32768", "PlotSpectrum", {"Algorithm": "Spectrum", "Function": "Gaussian(a=3.5) window", "Size": 32768, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_1_3_1024", "PlotSpectrum", {"Algorithm": "Standard Autocorrelation", "Function": "Hann window", "Size": 1024, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_2_3_2048", "PlotSpectrum", {"Algorithm": "Cuberoot Autocorrelation", "Function": "Hann window", "Size": 2048, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_3_2_1024", "PlotSpectrum", {"Algorithm": "Enhanced Autocorrelation", "Function": "Hamming window", "Size": 1024, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("PlotSpectrum_4_3_4096", "PlotSpectrum", {"Algorithm": "Cepstrum", "Function": "Hann window", "Size": 4096, "Axis": "Linear frequency"}, "PlotSpectrum", "ref_spectrum", 0, "ref: Audacity lib-fft.so SpectrumAnalyst + FreqWindow export format"),
 ("Contrast", "Contrast", {"FgStart": 0, "FgEnd": 1, "BgStart": 1, "BgEnd": 2}, "Contrast", "ref_contrast", 0, "ref: ContrastBase::GetDB formula (dialog-only); %.2f dB values + WCAG verdict"),
 ("ContrastQuiet", "Contrast", {"FgStart": 0.2, "FgEnd": 0.7, "BgStart": 0, "BgEnd": 0}, "Contrast", "ref_contrast", 0, "background = quiet aux track (ctl.wav x 0.01)"),
]

def write_wav(p, x):  # x: (ch, n) float32 -> 32-bit float WAV
    x = np.asarray(x, dtype="<f4"); ch, n = x.shape
    data = x.T.tobytes()
    with open(p, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 3, ch, SR, SR * ch * 4, ch * 4, 32) + b"data" + struct.pack("<I", len(data)) + data)

def read_wav(p):
    b = open(p, "rb").read(); i = 12; fmt = None
    while i < len(b):
        cid, sz = b[i:i+4], struct.unpack("<I", b[i+4:i+8])[0]
        if cid == b"fmt ":
            fmt = list(struct.unpack("<HHIIHH", b[i+8:i+24]))
            if fmt[0] == 0xFFFE: fmt[0] = struct.unpack("<H", b[i+32:i+34])[0]
        if cid == b"data":
            ch, bits = fmt[1], fmt[5]; tag = fmt[0]
            raw = b[i+8:i+8+sz]
            if tag == 3: d = np.frombuffer(raw, dtype="<f4" if bits == 32 else "<f8")
            elif bits == 16: d = np.frombuffer(raw, dtype="<i2") / 32768.0
            elif bits == 32: d = np.frombuffer(raw, dtype="<i4") / 2147483648.0
            elif bits == 24:
                a = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
                v = a[:, 0] | (a[:, 1] << 8) | (a[:, 2] << 16); v = np.where(v >= 1 << 23, v - (1 << 24), v); d = v / 8388608.0
            else: raise ValueError(f"unsupported wav {tag} {bits}")
            return d.reshape(-1, ch).T.astype(np.float64)
        i += 8 + sz + (sz & 1)
    raise ValueError(p)

def make_inputs():
    rng = np.random.default_rng(1234); t = np.arange(int(2.0 * SR)) / SR
    l = 0.4 * np.sin(2*np.pi*220*t) + 0.1 * np.sin(2*np.pi*3000*t) + 0.01 * rng.standard_normal(t.size)
    r = 0.3 * np.sin(2*np.pi*330*t + 1) + 0.05 * rng.standard_normal(t.size)
    env = np.ones_like(t); env[int(0.8*SR):int(1.2*SR)] = 0.002  # a near-silent gap
    l *= env; r *= env
    l[int(1.5*SR):int(1.5*SR)+5] = 1.0  # a few clipped samples + a click
    r[int(0.5*SR)+100] += 0.8
    l[:200] += 0.05  # small DC-ish start
    write_wav(f"{W}/in.wav", [l, r])
    c = np.zeros_like(t); c[int(0.5*SR):int(1.0*SR)] = 0.5 * np.sin(2*np.pi*500*t[int(0.5*SR):int(1.0*SR)])
    write_wav(f"{W}/ctl.wav", [c])
    # beats for BeatFinder: clicks every 0.5 s
    b = 0.02 * rng.standard_normal(t.size)
    for k in range(4): s = int((0.25 + 0.5*k) * SR); b[s:s+300] += np.exp(-np.arange(300)/50.0) * 0.9
    write_wav(f"{W}/beats.wav", [b])
    # clicks for ClickRemoval: quiet program material with short spikes
    k = 0.05 * np.sin(2*np.pi*440*t) + 0.002 * rng.standard_normal(t.size)
    k2 = 0.05 * np.sin(2*np.pi*660*t) + 0.002 * rng.standard_normal(t.size)
    for j, pos in enumerate([0.3, 0.71, 1.13, 1.52, 1.87]):
        s0 = int(pos*SR); k[s0:s0+2+j%3] += 0.7; k2[s0+500:s0+503] -= 0.6
    write_wav(f"{W}/clicks.wav", [k, k2])

PIPE_TO = f"/tmp/audacity_script_pipe.to.{os.getuid()}"; PIPE_FROM = f"/tmp/audacity_script_pipe.from.{os.getuid()}"
class Aud:
    def __enter__(self):
        # The pipe paths are per-uid, so only one scripted Audacity can run at a time.
        # Serialize with other agents/tests via a lock file and only ever kill our own process.
        import fcntl
        self.lock = open("/tmp/audacity_script_pipe.lock", "w"); fcntl.flock(self.lock, fcntl.LOCK_EX)
        for pth in (PIPE_TO, PIPE_FROM):  # stale pipes from a crashed run of ours (lock held => no live owner)
            if os.path.exists(pth) and not subprocess.run(["fuser", pth], capture_output=True).stdout.strip(): os.remove(pth)
        cfg = os.path.expanduser("~/.config/audacity/audacity.cfg"); s = open(cfg).read()
        s = re.sub(r"\[ActiveProjects\]\n(?:[^\[\n][^\n]*\n)*", "", s)
        s = re.sub(r"DefaultProjectSampleFormatChoice=.*", "DefaultProjectSampleFormatChoice=Format32BitFloat", s)
        open(cfg, "w").write(s)
        self.proc = subprocess.Popen("cd /tmp && exec xvfb-run -a -s '-screen 0 1280x1024x24' audacity >/tmp/aud.log 2>&1", shell=True, start_new_session=True)
        try:
            import stat
            for _ in range(90):
                if self.proc.poll() is not None: raise RuntimeError(f"Audacity exited with {self.proc.returncode} (see /tmp/aud.log)")
                if all(os.path.exists(p) and stat.S_ISFIFO(os.stat(p).st_mode) for p in (PIPE_TO, PIPE_FROM)): break
                time.sleep(1)
            else:
                raise TimeoutError("Audacity's script pipe did not appear within 90 s")
            time.sleep(3)
            self.to = open(PIPE_TO, "w"); self.fr = os.open(PIPE_FROM, os.O_RDONLY); self.buf = b""; return self
        except BaseException:
            # Failed start: kill our own Audacity (process group) and release the lock, else the
            # orphan keeps the per-uid pipe and blocks every later run on the box.
            self.__exit__(None, None, None)
            raise
    def __call__(self, c, timeout=120):
        import select
        self.to.write(c + "\n"); self.to.flush()
        end = time.time() + timeout
        while b"\n\n" not in self.buf:
            left = end - time.time()
            if left <= 0 or not select.select([self.fr], [], [], left)[0]:
                raise TimeoutError(f"Audacity did not answer '{c}' (crash/modal dialog)")
            chunk = os.read(self.fr, 65536)
            if not chunk: raise TimeoutError("pipe closed")
            self.buf += chunk
        r, self.buf = self.buf.split(b"\n\n", 1)
        r = r.decode(errors="replace")
        if "BatchCommand finished: OK" not in r: print("   audacity:", c, "->", r.strip()[-200:])
        return r
    def __exit__(self, *a):
        import signal, fcntl
        try: os.killpg(self.proc.pid, signal.SIGTERM)  # our xvfb-run + audacity only
        except ProcessLookupError: pass
        try: self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            try: os.killpg(self.proc.pid, signal.SIGKILL)
            except ProcessLookupError: pass
        time.sleep(1)
        for pth in (PIPE_TO, PIPE_FROM):
            if os.path.exists(pth): os.remove(pth)
        fcntl.flock(self.lock, fcntl.LOCK_UN); self.lock.close()

def labels_from(info):
    m = re.search(r"\[.*\]", info, re.S)
    out = []
    if not m: return out
    for tr in json.loads(m.group(0)):
        for lab in tr[1]: out.append((round(lab[0], 3), round(lab[1], 3), lab[2]))
    return out

def db(x): return 20*np.log10(max(x, 1e-12))

def rust(args):
    return subprocess.run([TOOL] + args, capture_output=True, text=True, check=True).stdout

def split_params(s):  # "Key=v with space Key2=v" -> ["Key=v with space", ...] using known-key boundaries
    if not s: return []
    parts = re.split(r" (?=[A-Za-z][A-Za-z0-9 \-]*?=)", s)
    return parts

def main():
    make_inputs()
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "au-app", "--example", "fx_tool"], cwd=R, check=True)
    cases = [c for c in CASES if not FILT or any(f == c[0] or (f.endswith("*") and c[0].startswith(f[:-1])) for f in FILT.split(","))]
    results = []
    A = Aud().__enter__()
    for name, cmd, params, rid, kind, thr, note in cases:
        ap_ = dict(params)
        if cmd == "Tone":
            # Audacity 3.7.3 scripting bug: Tone via mod-script-pipe skips PostSet(), so End* keep
            # stale values and the tone ramps. Tone == Chirp with equal start/end (same ToneGenBase
            # code path), which is exactly what the Tone dialog produces.
            cmd = "Chirp"
            ap_ = {"StartFreq": params["Frequency"], "EndFreq": params["Frequency"], "StartAmp": params["Amplitude"],
                   "EndAmp": params["Amplitude"], "Waveform": params["Waveform"], "Interpolation": "Linear"}
        acmd = cmd + ":" + "".join(f' {k.replace(" ", "_")}="{v}"' for k, v in ap_.items())  # Audacity macro syntax: spaces in keys -> _
        rp = [f"{k}={v}" for k, v in params.items()]
        try:
            run_case(A, name, acmd, rid, rp, kind, thr, note, results)
        except TimeoutError as e:
            row = {"case": name, "note": note, "thr": thr, "pass_": False, "error": f"AUDACITY: {e}"}
            results.append(row); print(json.dumps(row), flush=True)
            A.__exit__(); A = Aud().__enter__()
    A.__exit__()
    json.dump(results, open(f"{W}/results.json", "w"), indent=1)
    print(f"PASS {sum(r['pass_'] for r in results)}/{len(results)}")

def run_case(A, name, acmd, rid, rp, kind, thr, note, results):
    if True:
        if True:
            A("SelectAll:"); A("RemoveTracks:")
            if kind == "ref_classic":
                return run_classic_ref(name, rp, thr, note, results)
            if kind == "ref_spectrum":
                return run_spectrum_ref(name, rp, results, note)
            if kind == "ref_contrast":
                return run_contrast_ref(name, rp, results, note)
            if kind == "genfresh":
                A.__exit__(); A.__enter__()  # fresh process => glibc rand() default seed
                A("SelectAll:"); A("RemoveTracks:"); A("NewStereoTrack:"); A("SelectTime: Start=0 End=2 RelativeTo=ProjectStart")
                A(acmd)
                return finish_gen(A, name, rid, rp, thr, note, results)
            if kind == "txt":
                return run_txt(A, name, acmd, rid, rp, note, results)
            src = {"fx": "clicks.wav" if name == "ClickRemoval" else "in.wav", "aux": "in.wav", "lab": "beats.wav" if name == "BeatFinder" else "in.wav"}.get(kind)
            if src: A(f"Import2: Filename={W}/{src}")
            else: A("NewStereoTrack:"); A("SelectTime: Start=0 End=2 RelativeTo=ProjectStart")
            if kind == "aux":
                A(f"Import2: Filename={W}/ctl.wav"); A("SelectTracks: Track=0 TrackCount=1 Mode=Set")
                A("SelectTime: Start=0 End=2 RelativeTo=ProjectStart")
            elif src: A("SelectAll:")
            if kind == "gen" and cmd_is_gui(acmd):
                gui_generate(acmd)
            else:
                A(acmd)
            ap = f"{W}/{name}.aud.aup3"
            if os.path.exists(ap): os.remove(ap)
            info = A("GetInfo: Type=Labels Format=JSON") if kind == "lab" else ""
            A(f"SaveProject2: Filename={ap}")
            if kind == "gen" and cmd_is_gui(acmd): time.sleep(3)
            row = {"case": name, "note": note, "thr": thr}
            try:
                if kind == "lab":
                    out = rust(["analyze", rid, f"{W}/{src}"] + rp)
                    rl = [(round(float(a), 3), round(float(b), 3), c) for _, a, b, c in (l.split("\t") for l in out.splitlines() if l.startswith("LABEL"))]
                    al = labels_from(info)
                    ok = len(rl) == len(al) and all(abs(x[0]-y[0]) <= 0.002 and abs(x[1]-y[1]) <= 0.002 and x[2] == y[2] for x, y in zip(rl, al))
                    row.update(aud=len(al), rust=len(rl), pass_=ok, detail=f"aud {al[:6]} rust {rl[:6]}")
                else:
                    rust(["aup3wav", ap, f"{W}/{name}.aud.wav"])
                    a = read_wav(f"{W}/{name}.aud.wav")
                    if kind in ("gen", "gennoise"):
                        rust(["gen", rid, f"{W}/{name}.rs.wav", str(SR), "2", "2"] + rp)
                    else:
                        extra = [f"aux={W}/ctl.wav"] if kind == "aux" else []
                        rust(["apply", rid, f"{W}/{src}", f"{W}/{name}.rs.wav"] + rp + extra)
                    b = read_wav(f"{W}/{name}.rs.wav")
                    n = min(a.shape[1], b.shape[1]); ch = min(a.shape[0], b.shape[0])
                    d = a[:ch, :n] - b[:ch, :n]
                    rms_d = db(np.sqrt(np.mean(d**2))) if n else 0; pk = db(np.max(np.abs(d))) if n else 0
                    lvl_a, lvl_b = db(np.sqrt(np.mean(a**2))), db(np.sqrt(np.mean(b**2)))
                    len_ok = a.shape[1] == b.shape[1] and a.shape[0] == b.shape[0]
                    if thr == 0:  # stochastic: compare level within 1 dB and length
                        ok = len_ok and abs(lvl_a - lvl_b) < 1.0
                    else:
                        ok = len_ok and rms_d <= thr
                    row.update(len_aud=list(a.shape), len_rust=list(b.shape), rms_diff_db=round(rms_d, 1), peak_diff_db=round(pk, 1),
                               level_aud_db=round(lvl_a, 2), level_rust_db=round(lvl_b, 2), pass_=bool(ok))
            except Exception as e:
                row.update(pass_=False, error=str(e)[-300:])
            results.append(row)
            print(json.dumps(row), flush=True)

def finish_gen(A, name, rid, rp, thr, note, results):
    ap = f"{W}/{name}.aud.aup3"
    if os.path.exists(ap): os.remove(ap)
    A(f"SaveProject2: Filename={ap}")
    rust(["aup3wav", ap, f"{W}/{name}.aud.wav"]); a = read_wav(f"{W}/{name}.aud.wav")
    rust(["gen", rid, f"{W}/{name}.rs.wav", str(SR), str(a.shape[0]), "0"] + rp); b = read_wav(f"{W}/{name}.rs.wav")
    n = min(a.shape[1], b.shape[1]); ch = min(a.shape[0], b.shape[0]); d = a[:ch, :n] - b[:ch, :n]
    rms_d = db(np.sqrt(np.mean(d**2))) if n else 0
    ok = a.shape == b.shape and rms_d <= thr
    row = {"case": name, "note": note, "thr": thr, "len_aud": list(a.shape), "len_rust": list(b.shape), "rms_diff_db": round(rms_d, 1),
           "peak_diff_db": round(db(np.max(np.abs(d))), 1) if n else 0, "pass_": bool(ok)}
    results.append(row); print(json.dumps(row), flush=True)

def run_txt(A, name, acmd, rid, rp, note, results):
    """Nyquist analyzers that write a text file (Sample Data Export): compare the files byte-for-byte."""
    A(f"Import2: Filename={W}/in.wav"); A("SelectAll:")
    fa, fr = f"{W}/{name}.aud.txt", f"{W}/{name}.rs.txt"
    for f in (fa, fr):
        if os.path.exists(f): os.remove(f)
    A(acmd + f' FILENAME="{fa}"'); time.sleep(1)
    rust(["analyze", rid, f"{W}/in.wav"] + rp + [f"FILENAME={fr}"])
    ta = open(fa).read() if os.path.exists(fa) else None; tr = open(fr).read() if os.path.exists(fr) else None
    if tr: tr = tr.replace(fr, fa)  # header line carries the output file name
    ok = ta is not None and ta == tr
    row = {"case": name, "note": note, "thr": 0, "pass_": ok, "aud_lines": ta and ta.count("\n"), "rust_lines": tr and tr.count("\n")}
    if not ok and ta and tr:
        al, rl = ta.splitlines(), tr.splitlines()
        k = next((i for i, (x, y) in enumerate(zip(al, rl)) if x != y), min(len(al), len(rl)))
        row["first_diff"] = [k, al[k:k+1], rl[k:k+1]]
    results.append(row); print(json.dumps(row), flush=True)

SPEC_REF = "/tmp/plot_spectrum_ref"
def run_spectrum_ref(name, rp, results, note):
    """Plot Spectrum is dialog-only: reference = Audacity 3.7.3 lib-fft.so SpectrumAnalyst::Calculate on the
    channel sum (PlotSpectrumBase::GetAudio), printed in FreqWindow::OnExport's text format."""
    if not os.path.exists(SPEC_REF):
        subprocess.run(["g++", "-O1", "-o", SPEC_REF, os.path.join(R, "tools/aud_ref/plot_spectrum_ref.cpp"), "-L/usr/lib/audacity",
                        "-l:lib-fft.so", "-Wl,-rpath,/usr/lib/audacity"], check=True)
    _, a, w, n = name.split("_")
    x = read_wav(f"{W}/in.wav").astype(np.float32)
    raw = f"{W}/in.f32"; x.T.copy().astype("<f4").tofile(raw)
    ref = subprocess.run([SPEC_REF, raw, str(x.shape[0]), str(SR), a, w, n], capture_output=True, text=True).stdout
    out = rust(["analyze", "PlotSpectrum", f"{W}/in.wav"] + rp)
    al = ref.splitlines()
    rs = out[out.index(al[0]):] if al and al[0] in out else out
    rl = rs.splitlines()[:len(al)]
    exact = sum(x == y for x, y in zip(al, rl))
    try:
        av = np.array([[float(v) for v in l.split("\t")] for l in al[1:]]); rv = np.array([[float(v) for v in l.split("\t")] for l in rl[1:]])
        md = float(np.max(np.abs(av - rv))) if av.shape == rv.shape else None
    except Exception:
        md = None
    ok = len(rl) == len(al) and bool(al) and al[0] == rl[0] and md is not None and md <= 1e-4
    row = {"case": name, "note": note, "thr": 0, "rows": len(al) - 1, "identical_lines": exact, "max_abs_diff": md, "pass_": bool(ok)}
    results.append(row); print(json.dumps(row), flush=True)

def contrast_db(x, t0, t1):  # ContrastBase::GetDB
    s0, s1 = int(np.floor(t0*SR + 0.5)), int(np.floor(t1*SR + 0.5))
    if s1 <= s0: return None
    ms = sum(float(np.sum(c[s0:s1].astype(np.float64)**2)/(s1-s0)) for c in x)
    rms = np.sqrt(ms / x.shape[0]); return float(np.float32(20*np.log10(rms))) if rms > 0 else float("-inf")
def run_contrast_ref(name, rp, results, note):
    """Contrast is dialog-only: reference = ContrastBase::GetDB (per-channel WaveChannelUtilities::GetRMS,
    mean of squares over channels, LINEAR_TO_DB) and Contrast.cpp's %.2f formatting / WCAG 20 dB rule."""
    p = {k: float(v) for k, v in (z.split("=", 1) for z in rp)}
    x = read_wav(f"{W}/in.wav")
    quiet = p["BgEnd"] <= p["BgStart"]
    if quiet:
        write_wav(f"{W}/quiet.wav", (read_wav(f"{W}/ctl.wav") * 0.01).astype(np.float32))
    bgx = read_wav(f"{W}/quiet.wav") if quiet else x
    fg = contrast_db(x, p["FgStart"], p["FgEnd"]); bg = contrast_db(bgx, 0 if quiet else p["BgStart"], 2 if quiet else p["BgEnd"])
    diff = fg - bg
    exp = [f"{fg:.2f} dB", f"{bg:.2f} dB", f"{diff:.2f}", "Pass" if diff > 20 else "Fail"]
    out = rust(["analyze", "Contrast", f"{W}/in.wav"] + rp + ([f"aux={W}/quiet.wav"] if quiet else []))
    got = [s in out for s in exp]
    row = {"case": name, "note": note, "thr": 0, "expected": exp, "found": got, "pass_": all(got)}
    if not all(got): row["rust_out"] = out[-600:]
    results.append(row); print(json.dumps(row), flush=True)

def cmd_is_gui(acmd): return acmd.startswith("DtmfTones:")
def gui_generate(acmd):
    """Audacity 3.7.3 DTMF via scripting ignores Sequence/Duty Cycle and computes tone lengths from a stale
    duration (DtmfSettings::Recalculate runs before the selection duration is applied). So drive the real
    dialog: Generate > DTMF Tones... and press OK with the dialog defaults (sequence 'audacity', 55 %, 0.8),
    duration = the 2 s selection."""
    import shlex
    pid = subprocess.run("pgrep -x audacity", shell=True, capture_output=True, text=True).stdout.split()[0]
    envs = dict(l.split("=", 1) for l in open(f"/proc/{pid}/environ").read().split("\0") if "=" in l)
    e = dict(os.environ, DISPLAY=envs["DISPLAY"], XAUTHORITY=envs.get("XAUTHORITY", ""))
    x = lambda *a: subprocess.run(["xdotool", *a], env=e, capture_output=True)
    x("search", "--onlyvisible", "--name", "Welcome to Audacity", "windowactivate", "--sync", "key", "Escape"); time.sleep(1)
    x("search", "--onlyvisible", "--name", "^Audacity$", "windowactivate", "--sync"); time.sleep(0.5)
    x("key", "alt+g"); time.sleep(1); x("key", "Down", "Down", "Return"); time.sleep(3)
    wid = x("search", "--onlyvisible", "--name", "^DTMF Tones$").stdout.split()[-1].decode()
    geo = x("getwindowgeometry", wid).stdout.decode()
    w, h = map(int, re.search(r"Geometry: (\d+)x(\d+)", geo).groups())
    x("mousemove", "--window", wid, str(w - 52), str(h - 21), "click", "1"); time.sleep(4)  # 'Generate' button (bottom-right)

CF_TYPES = ["Butterworth", "Chebyshev Type I", "Chebyshev Type II"]; CF_SUB = ["Lowpass", "Highpass"]
def run_classic_ref(name, rp, thr, note, results):
    """Audacity 3.7.3 crashes running Classic Filters from scripts (null mpBiquad in ProcessInitialize),
    so the reference is Audacity's own lib-math.so driven by tools/aud_ref/classic_filters_ref.cpp."""
    ref = "/tmp/classic_filters_ref"
    if not os.path.exists(ref):
        subprocess.run(["g++", "-O2", "-o", ref, os.path.join(R, "tools/aud_ref/classic_filters_ref.cpp"), "-L/usr/lib/audacity",
                        "-l:lib-math.so", "-Wl,-rpath,/usr/lib/audacity"], check=True)
    p = dict(x.split("=", 1) for x in rp)
    args = [str(CF_TYPES.index(p["FilterType"])), str(CF_SUB.index(p["FilterSubtype"])), p["Order"], p["Cutoff"],
            p.get("PassbandRipple", "1"), p.get("StopbandRipple", "30"), str(SR)]
    x = read_wav(f"{W}/in.wav").astype(np.float32)
    a = np.stack([np.frombuffer(subprocess.run([ref] + args, input=ch.tobytes(), capture_output=True, check=True).stdout, dtype="<f4") for ch in x]).astype(np.float64)
    rust(["apply", "ClassicFilters", f"{W}/in.wav", f"{W}/{name}.rs.wav"] + rp)
    b = read_wav(f"{W}/{name}.rs.wav")
    d = a - b[:, :a.shape[1]] if b.shape == a.shape else None
    row = {"case": name, "note": note, "thr": thr, "len_aud": list(a.shape), "len_rust": list(b.shape)}
    if d is None: row.update(pass_=False)
    else:
        rms_d = db(np.sqrt(np.mean(d**2))); row.update(rms_diff_db=round(rms_d, 1), peak_diff_db=round(db(np.max(np.abs(d))), 1), pass_=bool(rms_d <= thr))
    results.append(row); print(json.dumps(row), flush=True)

if __name__ == "__main__":
    main()
