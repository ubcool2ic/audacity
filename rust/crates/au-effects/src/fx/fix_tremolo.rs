use crate::core::*;
use au_core::AudioBuffer;

const WAVEFORMS: &[&str] = &[
    "Sine",
    "Triangle",
    "Sawtooth",
    "Inverse Sawtooth",
    "Square",
];

/// Audacity's Nyquist Tremolo effect.
pub struct Tremolo {
    /// Waveform choice index.
    pub wave: usize,
    /// Starting phase in degrees.
    pub phase: i32,
    /// Wet level in percent.
    pub wet: i32,
    /// LFO frequency in hertz.
    pub lfo: f64,
}

impl Default for Tremolo {
    fn default() -> Self {
        Self {
            wave: 0,
            phase: 0,
            wet: 40,
            lfo: 4.0,
        }
    }
}

impl BuiltinEffect for Tremolo {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Tremolo"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("WAVE", "Waveform type", WAVEFORMS, 0),
            ParamDef::int("PHASE", "Starting phase (degrees)", -180.0, 180.0, 0.0),
            ParamDef::int("WET", "Wet level (percent)", 0.0, 100.0, 40.0),
            ParamDef::float("LFO", "Frequency (Hz)", 0.001, 1000.0, 4.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "WAVE" => Some(self.wave as f64),
            "PHASE" => Some(self.phase as f64),
            "WET" => Some(self.wet as f64),
            "LFO" => Some(self.lfo),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "WAVE" => {
                self.wave = ParamDef::choice("WAVE", "", WAVEFORMS, 0)
                    .clamp(value) as usize;
                true
            }
            "PHASE" => {
                self.phase = ParamDef::int("PHASE", "", -180.0, 180.0, 0.0)
                    .clamp(value) as i32;
                true
            }
            "WET" => {
                self.wet =
                    ParamDef::int("WET", "", 0.0, 100.0, 40.0).clamp(value) as i32;
                true
            }
            "LFO" => {
                self.lfo = ParamDef::float("LFO", "", 0.001, 1000.0, 4.0).clamp(value);
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.sample_rate == 0 {
            return Ok(());
        }

        let sample_rate = buf.sample_rate as f64;
        let control_rate = sample_rate / 20.0;
        let wave = self.wave.min(WAVEFORMS.len() - 1);
        let table = waveform_table(wave, control_rate);
        let table_len = (table.len() - 1) as f64;
        if table_len <= 0.0 {
            return Ok(());
        }

        let phase_degrees = if wave == 0 {
            self.phase as f64 - 90.0
        } else {
            self.phase as f64
        };
        let mut phase = phase_degrees * table_len / 360.0;
        while phase < 0.0 {
            phase += table_len;
        }
        while phase >= table_len {
            phase -= table_len;
        }

        let phase_increment = self.lfo * table_len / sample_rate;
        let wet = self.wet as f64 / 200.0;
        let dry_gain = (1.0 - wet) as f32;
        let wet_gain = wet as f32;

        for channel in &mut buf.channels {
            let mut oscillator_phase = phase;
            for sample in channel {
                let index = oscillator_phase as usize;
                let fraction = oscillator_phase - index as f64;
                let first = table[index] as f64;
                let oscillator = (first
                    + fraction * (table[index + 1] as f64 - first))
                    as f32;

                // Nyquist's sum/mult operations write sample_type (float) blocks.
                let modulation = dry_gain + wet_gain * oscillator;
                *sample *= modulation;

                oscillator_phase += phase_increment;
                while oscillator_phase >= table_len {
                    oscillator_phase -= table_len;
                }
            }
        }

        Ok(())
    }
}

/// Build a Nyquist oscillator table, including its interpolation guard sample.
fn waveform_table(wave: usize, control_rate: f64) -> Vec<f32> {
    match wave {
        0 => sine_table(),
        1 => triangle_table(),
        2 => pwlv_table(control_rate, &[(-1.0, 0.0), (1.0, 0.995), (-1.0, 1.0)]),
        3 => pwlv_table(control_rate, &[(-1.0, 0.0), (1.0, 0.005), (-1.0, 1.0)]),
        4 => pwlv_table(
            control_rate,
            &[
                (-1.0, 0.0),
                (1.0, 0.005),
                (1.0, 0.5),
                (-1.0, 0.505),
                (-1.0, 1.0),
            ],
        ),
        _ => sine_table(),
    }
}

/// Nyquist's 2048-point, single-harmonic sine table with a guard point.
fn sine_table() -> Vec<f32> {
    const N: usize = 2048;
    let mut table = Vec::with_capacity(N + 1);
    for i in 0..N {
        let angle = std::f64::consts::TAU * i as f64 / N as f64;
        table.push(angle.sin() as f32);
    }
    table.push(table[0]);
    table
}

/// Build a 2048-point triangle table with its initial value at -1.
fn triangle_table() -> Vec<f32> {
    const N: usize = 2048;
    let mut table = Vec::with_capacity(N + 1);
    for i in 0..N {
        let position = i as f64 / N as f64;
        let value = if position < 0.5 {
            -1.0 + 4.0 * position
        } else {
            3.0 - 4.0 * position
        };
        table.push(value as f32);
    }
    table.push(table[0]);
    table
}

/// Reproduce a one-second Nyquist `pwlv` sound sampled at control rate.
fn pwlv_table(control_rate: f64, points: &[(f64, f64)]) -> Vec<f32> {
    if !control_rate.is_finite() || control_rate <= 0.0 {
        return vec![-1.0, -1.0];
    }

    let count = (control_rate + 0.5).floor() as usize;
    let mut table = vec![0.0_f32; count];

    if points.is_empty() || count == 0 {
        table.push(table.first().copied().unwrap_or(0.0));
        return table;
    }

    let sample_index = |time: f64| -> usize {
        (time * control_rate + 0.5).floor().max(0.0) as usize
    };

    let mut current_index = sample_index(points[0].1);
    let mut current_level = points[0].0;
    let mut output_index = 0usize;

    for &(target_level, target_time) in points.iter().skip(1) {
        let target_index = sample_index(target_time).min(count);
        if target_index <= current_index {
            current_level = target_level;
            current_index = target_index;
            continue;
        }

        let segment_length = (target_index - current_index) as f64;
        let increment = (target_level - current_level) / segment_length;
        let mut remaining = target_index - current_index;

        // Nyquist pwl updates its stored level once per output block, while
        // each sample within a block uses a local incrementing level.
        while remaining > 0 {
            let block_len = remaining.min(1024);
            let mut level = current_level;
            for _ in 0..block_len {
                if output_index < count {
                    table[output_index] = level as f32;
                    output_index += 1;
                }
                level += increment;
            }
            current_level += increment * block_len as f64;
            current_index += block_len;
            remaining -= block_len;
        }

        // At a breakpoint, pwl loads the specified level for the next segment.
        current_level = target_level;
        current_index = target_index;
    }

    while output_index < count {
        table[output_index] = current_level as f32;
        output_index += 1;
    }

    table.push(table.first().copied().unwrap_or(0.0));
    table
}

/// Return the Tremolo effect as a built-in effect.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Tremolo::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn sine_buffer() -> AudioBuffer {
        let sample_rate = 44_100;
        let channels = (0..2)
            .map(|channel| {
                (0..256)
                    .map(|i| {
                        let phase = std::f64::consts::TAU * 440.0 * i as f64
                            / sample_rate as f64;
                        (phase.sin() * if channel == 0 { 0.5 } else { 0.25 }) as f32
                    })
                    .collect()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, channels)
    }

    #[test]
    fn default_processes_stereo_without_panic() {
        let mut effect = Tremolo::default();
        let mut buffer = sine_buffer();
        let mut context = EffectContext::default();
        effect.process(&mut buffer, &mut context).unwrap();
        assert_eq!(buffer.channels.len(), 2);
        assert_eq!(buffer.channels[0].len(), 256);
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = Tremolo::default();
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();
        effect.process(&mut buffer, &mut context).unwrap();
        assert!(buffer.channels.iter().all(Vec::is_empty));
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = sine_buffer();
        let mut second = sine_buffer();
        let mut context = EffectContext::default();

        Tremolo::default()
            .process(&mut first, &mut context)
            .unwrap();
        Tremolo::default()
            .process(&mut second, &mut context)
            .unwrap();

        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn set_param_clamps_values() {
        let mut effect = Tremolo::default();
        assert!(set_param(&mut effect, "WAVE", 100.0));
        assert_eq!(effect.wave, WAVEFORMS.len() - 1);
        assert!(set_param(&mut effect, "PHASE", -1000.0));
        assert_eq!(effect.phase, -180);
        assert!(set_param(&mut effect, "WET", 101.0));
        assert_eq!(effect.wet, 100);
        assert!(set_param(&mut effect, "LFO", 0.0));
        assert_eq!(effect.lfo, 0.001);
        assert!(!set_param(&mut effect, "unknown", 1.0));
    }
}
