use crate::core::*;
use au_core::AudioBuffer;

/// Repairs a short damaged range using least-squares autoregressive interpolation.
pub struct Repair {
    /// The damaged range, expressed as `(start, end)` frame indices within each channel.
    /// An empty range is invalid and represents the default “no selection” state.
    pub selection: (usize, usize),
}

impl Default for Repair {
    fn default() -> Self {
        Self { selection: (0, 0) }
    }
}

fn linear_interpolate(buffer: &mut [f32], first: usize, count: usize) {
    let len = buffer.len();
    if count == 0 || first + count > len {
        return;
    }
    let decay = 0.9_f32;
    if first == 0 {
        if count + 1 >= len {
            return;
        }
        let mut delta = buffer[count] - buffer[count + 1];
        let mut value = buffer[count];
        for i in (0..count).rev() {
            value += delta;
            buffer[i] = value;
            value *= decay;
            delta *= decay;
        }
    } else if first + count == len {
        if first < 2 {
            return;
        }
        let mut delta = buffer[first - 1] - buffer[first - 2];
        let mut value = buffer[first - 1];
        for x in 0..count {
            value += delta;
            buffer[first + x] = value;
            value *= decay;
            delta *= decay;
        }
    } else {
        let left = buffer[first - 1];
        let right = buffer[first + count];
        let delta = (right - left) / (count + 1) as f32;
        for i in 0..count {
            buffer[first + i] = left + delta * (i + 1) as f32;
        }
    }
}

fn invert(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut m = vec![vec![0.0; 2 * n]; n];
    for i in 0..n {
        for j in 0..n {
            m[i][j] = a[i][j];
        }
        m[i][n + i] = 1.0;
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&x, &y| {
            m[x][col].abs().total_cmp(&m[y][col].abs())
        })?;
        if m[pivot][col].abs() < 1.0e-18 {
            return None;
        }
        m.swap(col, pivot);
        let d = m[col][col];
        for j in 0..2 * n {
            m[col][j] /= d;
        }
        for row in 0..n {
            if row != col {
                let factor = m[row][col];
                for j in 0..2 * n {
                    m[row][j] -= factor * m[col][j];
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n..].to_vec()).collect())
}

fn interpolate_audio(buffer: &mut [f32], first: usize, count: usize) {
    let n = buffer.len();
    if count == 0 || count >= n || first + count > n {
        return;
    }
    if first == 0 {
        buffer.reverse();
        interpolate_audio(buffer, n - count, count);
        buffer.reverse();
        return;
    }

    let context = first.saturating_sub(1).max(n.saturating_sub(first + count + 1));
    let p = (count.saturating_mul(3).min(50)).min(context) as usize;
    if p < 3 || p >= n {
        linear_interpolate(buffer, first, count);
        return;
    }

    let s: Vec<f64> = buffer.iter().map(|&x| x as f64).collect();
    let mut xmat = vec![vec![0.0_f64; p]; p];
    let mut b = vec![0.0_f64; p];
    for i in 0..n - p {
        if i + p < first || i >= first + count {
            for row in 0..p {
                for col in 0..p {
                    xmat[row][col] += s[i + row] * s[i + col];
                }
                b[row] += s[i + p] * s[i + row];
            }
        }
    }
    let Some(inv) = invert(&xmat) else {
        linear_interpolate(buffer, first, count);
        return;
    };
    let a: Vec<f64> = (0..p)
        .map(|row| (0..p).map(|col| inv[row][col] * b[col]).sum())
        .collect();

    let rows = n - p;
    let mut au = vec![vec![0.0; count]; rows];
    let mut ak = vec![vec![0.0; n - count]; rows];
    for row in 0..rows {
        for col in 0..p {
            let v = -a[col];
            let index = row + col;
            if index < first {
                ak[row][index] = v;
            } else if index >= first + count {
                ak[row][index - count] = v;
            } else {
                au[row][index - first] = v;
            }
        }
        let index = row + p;
        if index < first {
            ak[row][index] = 1.0;
        } else if index >= first + count {
            ak[row][index - count] = 1.0;
        } else {
            au[row][index - first] = 1.0;
        }
    }

    let mut normal = vec![vec![0.0; count]; count];
    let mut rhs = vec![0.0; count];
    for i in 0..count {
        for j in 0..count {
            normal[i][j] = (0..rows).map(|r| au[r][i] * au[r][j]).sum();
        }
        rhs[i] = (0..rows)
            .map(|r| {
                let known: f64 = (0..n - count).map(|k| ak[r][k] * if k < first { s[k] } else { s[k + count] }).sum();
                -au[r][i] * known
            })
            .sum();
    }
    let Some(normal_inv) = invert(&normal) else {
        linear_interpolate(buffer, first, count);
        return;
    };
    for i in 0..count {
        let value: f64 = (0..count).map(|j| normal_inv[i][j] * rhs[j]).sum();
        buffer[first + i] = value as f32;
    }
}

impl BuiltinEffect for Repair {
    fn id(&self) -> &'static str { "Repair" }

    fn params(&self) -> Vec<ParamDef> { vec![] }

    fn get(&self, _key: &str) -> Option<f64> { None }

    fn set(&mut self, _key: &str, _value: f64) -> bool { false }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        let (first, end) = self.selection;
        if first >= end || end > buf.len() || end - first > 128 {
            return Err(EffectError::Invalid("Repair requires a nonempty selection of at most 128 samples".into()));
        }
        for channel in &mut buf.channels {
            interpolate_audio(channel, first, end - first);
        }
        Ok(())
    }
}

/// Removes short, high-energy clicks from audio.
pub struct ClickRemoval {
    /// Click detection threshold (0–900).
    pub threshold: i32,
    /// Maximum click width (0–40).
    pub width: i32,
}

impl Default for ClickRemoval {
    fn default() -> Self { Self { threshold: 200, width: 20 } }
}

impl ClickRemoval {
    fn remove_clicks(&self, buffer: &mut [f32]) -> bool {
        const SEP: usize = 4096;
        let len = buffer.len();
        if len <= SEP || self.width == 0 || self.threshold == 0 {
            return false;
        }
        let mut energy: Vec<f32> = buffer.iter().map(|x| x * x).collect();
        let mut rms = energy.clone();
        let mut step = 1;
        while step < SEP {
            for i in 0..len - step {
                rms[i] += rms[i + step];
            }
            step *= 2;
        }
        for v in rms.iter_mut().take(len - SEP) {
            *v /= SEP as f32;
        }

        let mut changed = false;
        let mut left: Option<usize> = None;
        let mut reciprocal = self.width / 4;
        while reciprocal >= 1 {
            let ww = (self.width / reciprocal).max(1) as usize;
            for i in 0..len - SEP {
                if i + SEP / 2 + ww >= len {
                    break;
                }
                let msw = energy[i + SEP / 2..i + SEP / 2 + ww].iter().sum::<f32>() / ww as f32;
                if msw >= self.threshold as f32 * rms[i] / 10.0 {
                    if left.is_none() { left = Some(i + SEP / 2); }
                } else if let Some(l) = left {
                    if i as isize - l as isize + (SEP / 2) as isize <= (ww * 2) as isize {
                        let end = i + ww + SEP / 2;
                        if l < end && end < len {
                            let lv = buffer[l];
                            let rv = buffer[end];
                            let denom = (end - l) as f32;
                            for j in l..end {
                                buffer[j] = (rv * (j - l) as f32 + lv * (end - j) as f32) / denom;
                                energy[j] = buffer[j] * buffer[j];
                            }
                            changed = true;
                        }
                    }
                    left = None;
                }
            }
            reciprocal /= 2;
        }
        changed
    }
}

impl BuiltinEffect for ClickRemoval {
    fn id(&self) -> &'static str { "ClickRemoval" }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("Threshold", "Threshold", 0.0, 900.0, 200.0),
            ParamDef::int("Width", "Width", 0.0, 40.0, 20.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key { "Threshold" => Some(self.threshold as f64), "Width" => Some(self.width as f64), _ => None }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Threshold" => { self.threshold = value.round().clamp(0.0, 900.0) as i32; true }
            "Width" => { self.width = value.round().clamp(0.0, 40.0) as i32; true }
            _ => false,
        }
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        for channel in &mut buf.channels {
            if channel.len() <= 4096 {
                continue;
            }
            let mut pos = 0;
            while pos + 4096 < channel.len() {
                let end = (pos + 8192).min(channel.len());
                let mut window = vec![0.0; 8192];
                window[..end - pos].copy_from_slice(&channel[pos..end]);
                self.remove_clicks(&mut window);
                let count = end - pos;
                channel[pos..end].copy_from_slice(&window[..count]);
                pos += 4096;
            }
        }
        Ok(())
    }
}

/// Returns default instances of all effects in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Repair::default()), Box::new(ClickRemoval::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine() -> AudioBuffer {
        let channels = (0..2).map(|_| (0..9000).map(|i| (i as f32 * 0.03).sin()).collect()).collect();
        AudioBuffer::from_channels(44100, channels)
    }

    #[test]
    fn effects_smoke_repeat_and_empty() {
        let mut repair = Repair { selection: (4000, 4004) };
        let mut input = sine();
        let mut copy = input.channels.clone();
        repair.process(&mut input, &mut EffectContext::default()).unwrap();
        let mut input2 = AudioBuffer::from_channels(44100, copy.clone());
        repair.process(&mut input2, &mut EffectContext::default()).unwrap();
        assert_eq!(input.channels, input2.channels);

        let mut click = ClickRemoval::default();
        let mut a = sine();
        let mut b = sine();
        click.process(&mut a, &mut EffectContext::default()).unwrap();
        click.process(&mut b, &mut EffectContext::default()).unwrap();
        assert_eq!(a.channels, b.channels);

        let mut empty = AudioBuffer::from_channels(44100, vec![vec![], vec![]]);
        click.process(&mut empty, &mut EffectContext::default()).unwrap();
        assert!(set_param(&mut click, "Width", 500.0));
        assert_eq!(click.width, 40);
        assert!(set_param(&mut click, "Threshold", -1.0));
        assert_eq!(click.threshold, 0);
        copy.clear();
    }

    #[test]
    fn repair_interpolates_linear_fallback() {
        let mut effect = Repair { selection: (3, 5) };
        let mut buffer = AudioBuffer::from_channels(1, vec![vec![0.0, 1.0, 2.0, 99.0, 99.0, 5.0]]);
        effect.process(&mut buffer, &mut EffectContext::default()).unwrap();
        assert!((buffer.channels[0][3] - 3.0).abs() < 0.01);
        assert!((buffer.channels[0][4] - 4.0).abs() < 0.01);
    }

    #[test]
    fn click_noop_for_silence_and_small_buffer() {
        let mut effect = ClickRemoval::default();
        let mut buffer = AudioBuffer::from_channels(44100, vec![vec![0.0; 4096]]);
        effect.process(&mut buffer, &mut EffectContext::default()).unwrap();
        assert!(buffer.channels[0].iter().all(|&x| x == 0.0));
    }
}
