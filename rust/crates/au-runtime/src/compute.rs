// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline data-parallel kernels. Every kernel has a serial path and a
//! parallel path that produce **bit-identical** output: work is split only
//! along independent axes (tracks, sample chunks, summary blocks, FFT
//! frames) and every floating-point reduction keeps the serial order.
use crate::pool::{CancelToken, JobError, Progress};
use rayon::prelude::*;

/// How a kernel runs. `Parallel` uses the current rayon pool (wrap the call
/// in `JobCtx::install` / `Runtime::pool().install` to pick a pool).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exec { Serial, Parallel }

/// Order-preserving map over independent items (per-track effect render,
/// per-file import decode, per-track export encode). Cancellation is checked
/// before each item; progress advances by one per item.
pub fn map_items<I: Sync, O: Send>(
    exec: Exec,
    items: &[I],
    cancel: &CancelToken,
    progress: &Progress,
    f: impl Fn(&I) -> Result<O, JobError> + Sync,
) -> Result<Vec<O>, JobError> {
    let one = |i: &I| { cancel.check()?; let r = f(i); progress.add(1); r };
    match exec {
        Exec::Serial => items.iter().map(one).collect(),
        Exec::Parallel => items.par_iter().map(one).collect(),
    }
}

/// One track for offline mixing: mono or interleaved-free planar channels.
pub struct MixInput<'a> {
    /// Planar channels (1 = mono, spread to both outputs by pan law).
    pub channels: Vec<&'a [f32]>,
    pub gain: f32,
    /// -1 (left) .. 1 (right). Linear pan law, matching `au-io` playback.
    pub pan: f32,
    /// Start offset in output frames.
    pub offset: usize,
}

const MIX_CHUNK: usize = 16_384;

/// Offline stereo mix of `inputs` into `frames` frames (planar L/R).
///
/// Parallel over output chunks; inside each sample the tracks are summed in
/// track order, so the result equals the serial mix bit for bit.
pub fn mix_offline(
    exec: Exec,
    inputs: &[MixInput<'_>],
    frames: usize,
    cancel: &CancelToken,
    progress: &Progress,
) -> Result<[Vec<f32>; 2], JobError> {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let gains: Vec<(f32, f32)> = inputs
        .iter()
        .map(|t| {
            let p = t.pan.clamp(-1.0, 1.0);
            (t.gain * if p > 0.0 { 1.0 - p } else { 1.0 }, t.gain * if p < 0.0 { 1.0 + p } else { 1.0 })
        })
        .collect();
    let chunk = |start: usize, lc: &mut [f32], rc: &mut [f32]| -> Result<(), JobError> {
        cancel.check()?;
        for (t, &(gl, gr)) in inputs.iter().zip(&gains) {
            let (c0, c1) = (t.channels.first().copied(), t.channels.get(1).copied().or(t.channels.first().copied()));
            let (Some(c0), Some(c1)) = (c0, c1) else { continue };
            for i in 0..lc.len() {
                let f = start + i;
                if f < t.offset { continue; }
                let s = f - t.offset;
                if s >= c0.len() { break; }
                lc[i] += c0[s] * gl;
                rc[i] += c1.get(s).copied().unwrap_or(0.0) * gr;
            }
        }
        progress.add(lc.len() as u64);
        Ok(())
    };
    match exec {
        Exec::Serial => l
            .chunks_mut(MIX_CHUNK)
            .zip(r.chunks_mut(MIX_CHUNK))
            .enumerate()
            .try_for_each(|(k, (lc, rc))| chunk(k * MIX_CHUNK, lc, rc))?,
        Exec::Parallel => l
            .par_chunks_mut(MIX_CHUNK)
            .zip(r.par_chunks_mut(MIX_CHUNK))
            .enumerate()
            .try_for_each(|(k, (lc, rc))| chunk(k * MIX_CHUNK, lc, rc))?,
    }
    Ok([l, r])
}

/// Min / max / RMS of one waveform summary block (Audacity's 256- and
/// 64K-sample summaries).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary { pub min: f32, pub max: f32, pub rms: f32 }

fn summarize(b: &[f32]) -> Summary {
    let (mut mn, mut mx, mut sq) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f64);
    for &s in b { mn = mn.min(s); mx = mx.max(s); sq += (s as f64) * (s as f64); }
    Summary { min: mn, max: mx, rms: (sq / b.len().max(1) as f64).sqrt() as f32 }
}

/// Waveform summary with `block` samples per entry.
pub fn waveform_summary(exec: Exec, samples: &[f32], block: usize) -> Vec<Summary> {
    let block = block.max(1);
    match exec {
        Exec::Serial => samples.chunks(block).map(summarize).collect(),
        Exec::Parallel => samples.par_chunks(block).map(summarize).collect(),
    }
}

/// In-place radix-2 FFT (`re`, `im` length must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 { j ^= bit; bit >>= 1; }
        j |= bit;
        if i < j { re.swap(i, j); im.swap(i, j); }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f64).cos() as f32, (ang * k as f64).sin() as f32);
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr; im[b] = im[a] - ti;
                re[a] += tr; im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Hann-windowed magnitude spectrogram in dB: one row of `fft_size/2`
/// bins per hop. `fft_size` is rounded up to a power of two.
pub fn spectrogram(
    exec: Exec,
    samples: &[f32],
    fft_size: usize,
    hop: usize,
    cancel: &CancelToken,
) -> Result<Vec<Vec<f32>>, JobError> {
    let n = fft_size.max(2).next_power_of_two();
    let hop = hop.max(1);
    let window: Vec<f32> = (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32)
        .collect();
    let frames = if samples.is_empty() { 0 } else { (samples.len() - 1) / hop + 1 };
    let frame = |f: usize| -> Result<Vec<f32>, JobError> {
        if f % 64 == 0 { cancel.check()?; }
        let mut re: Vec<f32> = (0..n).map(|i| samples.get(f * hop + i).copied().unwrap_or(0.0) * window[i]).collect();
        let mut im = vec![0.0f32; n];
        fft(&mut re, &mut im);
        Ok((0..n / 2).map(|k| 10.0 * (re[k] * re[k] + im[k] * im[k] + 1e-20).log10()).collect())
    };
    match exec {
        Exec::Serial => (0..frames).map(frame).collect(),
        Exec::Parallel => (0..frames).into_par_iter().map(frame).collect(),
    }
}
