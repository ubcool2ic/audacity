#!/usr/bin/env python3
import glob
import os
import shutil
import struct
import subprocess
import sys
import time
import wave

import numpy as np


R = os.path.realpath(os.path.join(os.path.dirname(__file__), ".."))
W = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "/tmp/macrocmp")
TOOL = os.path.join(R, "target", "release", "examples", "fx_tool")
APP = os.path.join(R, "target", "release", "audacity-rs")
SAMPLE_RATE = 44100
MACROS = [("Fade Ends", "Macro_FadeEnds")]


def write_input(path):
    frames = SAMPLE_RATE * 4
    t = np.arange(frames, dtype=np.float64) / SAMPLE_RATE
    rng = np.random.default_rng(1)
    left = 0.5 * np.sin(2.0 * np.pi * 440.0 * t) + 0.1 * rng.standard_normal(frames)
    right = 0.4 * np.sin(2.0 * np.pi * 660.0 * t)
    samples = np.column_stack((left, right)).astype("<f4")

    data = samples.tobytes()
    fmt = struct.pack("<HHIIHH", 3, 2, SAMPLE_RATE, SAMPLE_RATE * 2 * 4, 2 * 4, 32)
    body = b"fmt " + struct.pack("<I", len(fmt)) + fmt
    body += b"data" + struct.pack("<I", len(data)) + data
    if len(data) & 1:
        body += b"\0"
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", len(body) + 4) + b"WAVE" + body)


def read_wav(path):
    with open(path, "rb") as f:
        raw = f.read()

    if len(raw) < 12 or raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise ValueError(f"{path}: not a RIFF/WAVE file")

    fmt = None
    data = None
    pos = 12
    while pos + 8 <= len(raw):
        chunk_id = raw[pos:pos + 4]
        size = struct.unpack_from("<I", raw, pos + 4)[0]
        start = pos + 8
        end = start + size
        if end > len(raw):
            raise ValueError(f"{path}: truncated RIFF chunk")
        chunk = raw[start:end]
        if chunk_id == b"fmt ":
            fmt = chunk
        elif chunk_id == b"data":
            data = chunk
        pos = end + (size & 1)

    if fmt is None or data is None or len(fmt) < 16:
        raise ValueError(f"{path}: missing fmt or data chunk")

    tag, channels, sample_rate, _, block_align, bits = struct.unpack_from("<HHIIHH", fmt)
    if tag == 0xFFFE:
        if len(fmt) < 40:
            raise ValueError(f"{path}: truncated extensible fmt chunk")
        tag = struct.unpack_from("<H", fmt, 24)[0]

    if channels < 1 or block_align < 1:
        raise ValueError(f"{path}: invalid channel count or block alignment")
    if len(data) % block_align:
        raise ValueError(f"{path}: data size is not a whole number of frames")

    frames = len(data) // block_align
    if tag == 3 and bits == 32:
        samples = np.frombuffer(data, dtype="<f4").copy()
    elif tag == 3 and bits == 64:
        samples = np.frombuffer(data, dtype="<f8").astype(np.float64)
    elif tag == 1 and bits == 16:
        samples = np.frombuffer(data, dtype="<i2").astype(np.float64) / 32768.0
    elif tag == 1 and bits == 24:
        b = np.frombuffer(data, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        values = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        values = (values ^ 0x800000) - 0x800000
        samples = values.astype(np.float64) / 8388608.0
    elif tag == 1 and bits == 32:
        samples = np.frombuffer(data, dtype="<i4").astype(np.float64) / 2147483648.0
    else:
        raise ValueError(f"{path}: unsupported WAV format tag={tag}, bits={bits}")

    if samples.size != frames * channels:
        raise ValueError(f"{path}: unexpected samples-per-frame layout")
    return samples.reshape(frames, channels), sample_rate


class Pipe:
    def __init__(self, to_path, from_path):
        self.to = open(to_path, "w", buffering=1)
        self.from_pipe = open(from_path, "r", buffering=1)

    def cmd(self, command):
        print(f">> {command}", file=sys.stderr, flush=True)
        self.to.write(command + "\n")
        self.to.flush()

        lines = []
        while True:
            line = self.from_pipe.readline()
            if line == "":
                break
            if not line.strip() and lines:
                break
            lines.append(line)
        reply = "".join(lines)
        print(reply[:200], file=sys.stderr, flush=True)
        return reply

    def close(self):
        for handle in (self.to, self.from_pipe):
            try:
                handle.close()
            except Exception:
                pass


def run_commands(pipe, input_path, project_path, macro_command):
    pipe.cmd(f"Import2: Filename={input_path}")
    pipe.cmd("SelectAll:")
    pipe.cmd(f"{macro_command}:")
    for stale in (project_path, project_path + "-wal", project_path + "-shm"):
        if os.path.exists(stale):
            os.remove(stale)
    pipe.cmd(f"SaveProject2: Filename={project_path} AddToHistory=0")


def convert_project(project_path, wav_path):
    subprocess.run([TOOL, "aup3wav", project_path, wav_path], check=True)


def dbfs(value):
    return 20.0 * np.log10(max(float(value), 1e-10))


def max_abs(samples):
    return float(np.max(np.abs(samples))) if samples.size else 0.0


def main():
    os.makedirs(W, exist_ok=True)
    input_path = os.path.join(W, "in.wav")
    write_input(input_path)
    input_samples, _ = read_wav(input_path)

    official_pipe = None
    official_env = os.environ.copy()
    official_env["DISPLAY"] = ":131"
    official_to = f"/tmp/audacity_script_pipe.to.{os.getuid()}"
    official_from = f"/tmp/audacity_script_pipe.from.{os.getuid()}"
    official_projects = {}

    try:
        subprocess.run(["/workspace/audacity-official/run.sh"], check=True, env=official_env)
        official_pipe = Pipe(official_to, official_from)
        for name, command in MACROS:
            macro_id = name.replace(" ", "")
            project_path = os.path.join(W, f"aud_{macro_id}.aup3")
            run_commands(official_pipe, input_path, project_path, command)
            official_projects[macro_id] = project_path
    finally:
        if official_pipe is not None:
            official_pipe.close()
        subprocess.run(
            ["pkill", "-f", "squashfs-root/bin/audacity"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        for path in glob.glob("/tmp/audacity_script_pipe.*"):
            try:
                os.remove(path)
            except OSError:
                pass

    rs_pipe_dir = os.path.join(W, "rspipe")
    if os.path.exists(rs_pipe_dir):
        shutil.rmtree(rs_pipe_dir)
    os.makedirs(rs_pipe_dir)
    rs_to = os.path.join(rs_pipe_dir, "audacity_script_pipe.to")
    rs_from = os.path.join(rs_pipe_dir, "audacity_script_pipe.from")
    rs_env = os.environ.copy()
    rs_env["AU_SCRIPT_PIPE"] = "1"
    rs_env["AU_SCRIPT_PIPE_DIR"] = rs_pipe_dir
    rs_env["DISPLAY"] = os.environ.get("DISPLAY", ":10")
    rs_process = None
    rs_pipe = None
    rs_projects = {}

    try:
        rs_process = subprocess.Popen([APP], env=rs_env)
        deadline = time.time() + 30.0
        while time.time() < deadline and not (os.path.exists(rs_to) and os.path.exists(rs_from)):
            if rs_process.poll() is not None:
                raise RuntimeError(f"audacity-rs exited with status {rs_process.returncode}")
            time.sleep(0.1)
        if not (os.path.exists(rs_to) and os.path.exists(rs_from)):
            raise TimeoutError("Timed out waiting for audacity-rs script-pipe files")

        rs_pipe = Pipe(rs_to, rs_from)
        for name, command in MACROS:
            macro_id = name.replace(" ", "")
            project_path = os.path.join(W, f"rs_{macro_id}.aup3")
            run_commands(rs_pipe, input_path, project_path, command)
            rs_projects[macro_id] = project_path
    finally:
        if rs_pipe is not None:
            rs_pipe.close()
        if rs_process is not None:
            rs_process.terminate()
            try:
                rs_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                rs_process.kill()
                rs_process.wait()

    failed = False
    for name, _ in MACROS:
        macro_id = name.replace(" ", "")
        aud_wav = os.path.join(W, f"aud_{macro_id}.wav")
        rs_wav = os.path.join(W, f"rs_{macro_id}.wav")
        convert_project(official_projects[macro_id], aud_wav)
        convert_project(rs_projects[macro_id], rs_wav)

        aud, aud_rate = read_wav(aud_wav)
        rs, rs_rate = read_wav(rs_wav)
        same_shape = aud.shape == rs.shape
        if same_shape:
            difference = aud.astype(np.float64) - rs.astype(np.float64)
            peak_difference = max_abs(difference)
            rms_difference = float(np.sqrt(np.mean(difference * difference))) if difference.size else 0.0
        else:
            peak_difference = float("inf")
            rms_difference = float("inf")

        input_changed = False
        input_delta = float("nan")
        if aud.shape == input_samples.shape:
            input_delta = max_abs(aud.astype(np.float64) - input_samples.astype(np.float64))
            input_changed = input_delta > 0.01

        aud_frames = aud.shape[0]
        rs_frames = rs.shape[0]
        aud_edge = min(SAMPLE_RATE // 2, aud_frames)
        rs_edge = min(SAMPLE_RATE // 2, rs_frames)

        print(f"\n{name} ({macro_id})")
        print(
            f"  Audacity: frames={aud_frames}, channels={aud.shape[1]}, "
            f"rate={aud_rate}, length={aud_frames / aud_rate:.6f}s"
        )
        print(
            f"  audacity-rs: frames={rs_frames}, channels={rs.shape[1]}, "
            f"rate={rs_rate}, length={rs_frames / rs_rate:.6f}s"
        )
        print(f"  Same shape: {same_shape}")
        print(f"  Max |diff|: {peak_difference:.9g}")
        print(f"  RMS diff: {dbfs(rms_difference):.2f} dBFS")
        print(
            f"  Audacity peak |sample|, first/last 0.5s: "
            f"{max_abs(aud[:aud_edge]):.9g} / {max_abs(aud[-aud_edge:]):.9g}"
        )
        print(
            f"  audacity-rs peak |sample|, first/last 0.5s: "
            f"{max_abs(rs[:rs_edge]):.9g} / {max_abs(rs[-rs_edge:]):.9g}"
        )
        print(f"  Input changed (max |aud - in| = {input_delta:.9g} > 0.01): {input_changed}")

        passed = same_shape and peak_difference < 1e-6
        print(f"  {'PASS' if passed else 'FAIL'}")
        failed |= not passed

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
