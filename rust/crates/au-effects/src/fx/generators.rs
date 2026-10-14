use crate::core::*;
use au_core::AudioBuffer;
use std::f64::consts::PI;

/// Waveforms supported by Tone and Chirp.
const WAVEFORMS: &[&str] = &["Sine", "Square", "Sawtooth", "Square, no alias", "Triangle"];
/// Tone and Chirp interpolation modes.
const INTERPOLATIONS: &[&str] = &["Linear", "Logarithmic"];

fn duration_frames(buf: &AudioBuffer, duration: f64) -> usize {
    if buf.len() == 0 {
        (duration.max(0.0) * buf.sample_rate as f64).round() as usize
    } else {
        buf.len()
    }
}

fn resize_generated(buf: &mut AudioBuffer, frames: usize, data: &[f32]) {
    let channels = buf.channels.len();
    for ch in &mut buf.channels {
        ch.resize(frames, 0.0);
        if !data.is_empty() {
            ch.copy_from_slice(data);
        }
    }
    if channels == 0 {
        buf.channels.clear();
    }
}

fn tone_sample(
    wave: usize,
    phase: f64,
    frequency: f64,
    sample_rate: f64,
) -> f64 {
    let cycles = phase / sample_rate;
    match wave {
        1 => if cycles.fract() < 0.5 { 1.0 } else { -1.0 },
        2 => 2.0 * (cycles + 0.5).fract() - 1.0,
        3 => {
            let two_pi = 2.0 * PI;
            let scale = 4.0 / PI;
            let b = (1.0 + (two_pi * frequency / sample_rate).cos()) / scale;
            let mut v = scale * (two_pi * cycles).sin();
            let mut k = 3;
            while k < 200 && (k as f64) * frequency < sample_rate / 2.0 {
                let a = 1.0 + (two_pi * (k as f64) * frequency / sample_rate).cos();
                v += a * (two_pi * cycles * (k as f64)).sin() / (b * k as f64);
                k += 2;
            }
            v
        }
        4 => {
            let mut v = cycles.fract();
            if v < 0.25 {
                v *= 4.0;
            } else if v > 0.75 {
                v = (v - 1.0) * 4.0;
            } else {
                v = (0.5 - v) * 4.0;
            }
            v
        }
        _ => (2.0 * PI * cycles).sin(),
    }
}

fn tone_data(
    frames: usize,
    sample_rate: u32,
    wave: usize,
    interp: usize,
    f0: f64,
    f1: f64,
    a0: f64,
    a1: f64,
) -> Vec<f32> {
    // Audacity's dialog validates frequencies below Nyquist; enforce it here too.
    let nyq = sample_rate.max(2) as f64 / 2.0;
    let (f0, f1) = (f0.clamp(0.0, nyq), f1.clamp(0.0, nyq));
    let n = frames as f64;
    let mut phase = 0.0;
    let mut output = Vec::with_capacity(frames);
    let (mut freq, fq, mut log_freq) = if interp == 1 {
        let l0 = f0.max(1.0).log10();
        let l1 = f1.max(1.0).log10();
        (10f64.powf(l0), (l1 - l0) / n.max(1.0), l0)
    } else {
        (f0, (f1 - f0) / n.max(1.0), 0.0)
    };
    let aq = (a1 - a0) / n.max(1.0);
    let mut amp = a0;
    for _ in 0..frames {
        output.push((amp * tone_sample(wave, phase, freq, sample_rate as f64)) as f32);
        phase += freq;
        amp += aq;
        if interp == 1 {
            log_freq += fq;
            freq = 10f64.powf(log_freq);
        } else {
            freq += fq;
        }
    }
    output
}

/// Generate a tone.
#[derive(Clone, Debug)]
pub struct Tone {
    /// Frequency in Hz.
    pub frequency: f64,
    /// Linear amplitude in the range 0–1.
    pub amplitude: f64,
    /// Waveform selection.
    pub waveform: usize,
    /// Frequency interpolation selection.
    pub interpolation: usize,
    /// Output duration in seconds when generating into an empty buffer.
    pub duration: f64,
}
impl Default for Tone {
    fn default() -> Self {
        Self { frequency: 440.0, amplitude: 0.8, waveform: 0, interpolation: 0, duration: 30.0 }
    }
}
impl BuiltinEffect for Tone {
    fn id(&self) -> &'static str { "Tone" }
    fn kind(&self) -> EffectKind { EffectKind::Generate }
    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Frequency", "Frequency", 1.0, f64::MAX, 440.0),
            ParamDef::float("Amplitude", "Amplitude", 0.0, 1.0, 0.8),
            ParamDef::choice("Waveform", "Waveform", WAVEFORMS, 0),
            ParamDef::choice("Interpolation", "Interpolation", INTERPOLATIONS, 0),
            ParamDef::float("Duration", "Duration", 0.0, 3600.0, 30.0),
        ]
    }
    fn get(&self, k: &str) -> Option<f64> {
        match k { "Frequency" => Some(self.frequency), "Amplitude" => Some(self.amplitude),
            "Waveform" => Some(self.waveform as f64), "Interpolation" => Some(self.interpolation as f64),
            "Duration" => Some(self.duration), _ => None }
    }
    fn set(&mut self, k: &str, v: f64) -> bool {
        match k { "Frequency" => self.frequency=v, "Amplitude" => self.amplitude=v,
            "Waveform" => self.waveform=v as usize, "Interpolation" => self.interpolation=v as usize,
            "Duration" => self.duration=v, _ => return false } true
    }
    fn process(&mut self, b: &mut AudioBuffer, _: &mut EffectContext) -> Result<(), EffectError> {
        let n = duration_frames(b, self.duration);
        let d = tone_data(n, b.sample_rate, self.waveform, self.interpolation,
            self.frequency, self.frequency, self.amplitude, self.amplitude);
        resize_generated(b, n, &d); Ok(())
    }
}

/// Generate a frequency- and amplitude-swept tone.
#[derive(Clone, Debug)]
pub struct Chirp {
    /// Starting frequency in Hz.
    pub start_freq: f64,
    /// Ending frequency in Hz.
    pub end_freq: f64,
    /// Starting linear amplitude.
    pub start_amp: f64,
    /// Ending linear amplitude.
    pub end_amp: f64,
    /// Waveform selection.
    pub waveform: usize,
    /// Frequency interpolation selection.
    pub interpolation: usize,
    /// Output duration in seconds when generating into an empty buffer.
    pub duration: f64,
}
impl Default for Chirp {
    fn default() -> Self {
        Self { start_freq: 440.0, end_freq: 1320.0, start_amp: 0.8, end_amp: 0.1,
            waveform: 0, interpolation: 0, duration: 30.0 }
    }
}
impl BuiltinEffect for Chirp {
    fn id(&self) -> &'static str { "Chirp" }
    fn kind(&self) -> EffectKind { EffectKind::Generate }
    fn params(&self) -> Vec<ParamDef> { vec![
        ParamDef::float("StartFreq", "Start frequency", 1.0, f64::MAX, 440.0),
        ParamDef::float("EndFreq", "End frequency", 1.0, f64::MAX, 1320.0),
        ParamDef::float("StartAmp", "Start amplitude", 0.0, 1.0, 0.8),
        ParamDef::float("EndAmp", "End amplitude", 0.0, 1.0, 0.1),
        ParamDef::choice("Waveform", "Waveform", WAVEFORMS, 0),
        ParamDef::choice("Interpolation", "Interpolation", INTERPOLATIONS, 0),
        ParamDef::float("Duration", "Duration", 0.0, 3600.0, 30.0),
    ] }
    fn get(&self,k:&str)->Option<f64>{match k{
        "StartFreq"=>Some(self.start_freq),"EndFreq"=>Some(self.end_freq),"StartAmp"=>Some(self.start_amp),
        "EndAmp"=>Some(self.end_amp),"Waveform"=>Some(self.waveform as f64),
        "Interpolation"=>Some(self.interpolation as f64),"Duration"=>Some(self.duration),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{
        "StartFreq"=>self.start_freq=v,"EndFreq"=>self.end_freq=v,"StartAmp"=>self.start_amp=v,
        "EndAmp"=>self.end_amp=v,"Waveform"=>self.waveform=v as usize,
        "Interpolation"=>self.interpolation=v as usize,"Duration"=>self.duration=v,_=>return false}true}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{
        let n=duration_frames(b,self.duration);
        let d=tone_data(n,b.sample_rate,self.waveform,self.interpolation,self.start_freq,self.end_freq,self.start_amp,self.end_amp);
        resize_generated(b,n,&d);Ok(())
    }
}

/// Noise generator. Its seeded RNG is deterministic; Audacity's use of `rand()` is not.
#[derive(Clone, Debug)]
pub struct Noise {
    /// Noise color: white, pink, or Brownian.
    pub noise_type: usize,
    /// Linear output amplitude.
    pub amplitude: f64,
    /// Seed for the generator's deterministic pseudo-random sequence.
    pub seed: u64,
    /// Output duration in seconds when generating into an empty buffer.
    pub duration: f64,
}
impl Default for Noise {
    fn default()->Self{Self{noise_type:0,amplitude:0.8,seed:1,duration:30.0}}
}
impl BuiltinEffect for Noise {
    fn id(&self)->&'static str{"Noise"}
    fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![
        ParamDef::choice("Type","Noise type",&["White","Pink","Brownian"],0),
        ParamDef::float("Amplitude","Amplitude",0.0,1.0,0.8),
        ParamDef::float("Duration","Duration",0.0,3600.0,30.0)]}
    fn get(&self,k:&str)->Option<f64>{match k{"Type"=>Some(self.noise_type as f64),"Amplitude"=>Some(self.amplitude),"Duration"=>Some(self.duration),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{"Type"=>self.noise_type=v as usize,"Amplitude"=>self.amplitude=v,"Duration"=>self.duration=v,_=>return false}true}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{
        let n=duration_frames(b,self.duration);let mut rng=self.seed;let(mut p0,mut p1,mut p2,mut p3,mut p4,mut p5,mut p6,mut y)=(0f32,0f32,0f32,0f32,0f32,0f32,0f32,0f32);
        let sr=b.sample_rate.max(1) as f32;
        let mut out=Vec::with_capacity(n);
        for _ in 0..n {
            rng^=rng<<13;rng^=rng>>7;rng^=rng<<17;
            let w=((rng as u32) as f32 / u32::MAX as f32)*2.0-1.0;
            let x=match self.noise_type {
                1=>{p0=0.99886*p0+0.0555179*w;p1=0.99332*p1+0.0750759*w;p2=0.969*p2+0.153852*w;p3=0.8665*p3+0.3104856*w;p4=0.55*p4+0.5329522*w;p5=-0.7616*p5-0.016898*w;let x=0.129*(p0+p1+p2+p3+p4+p5+p6+w*0.5362);p6=w*0.115926;x},
                2=>{let leakage=((sr-144.0)/sr).min(0.9999);let scale=(9.0/sr.sqrt()).max(0.01);let z=leakage*y+w*scale;y=if z.abs()>1.0{leakage*y-w*scale}else{z};y},
                _=>w
            };
            out.push((self.amplitude as f32)*x);
        }
        resize_generated(b,n,&out);Ok(())
    }
}

/// DTMF Tones (port of Audacity 3.7.3 `DtmfBase.cpp`).
#[derive(Clone, Debug)]
pub struct Dtmf {
    /// Sequence of keypad symbols (0-9 A-D a-z * #).
    pub sequence: String,
    /// Duty cycle in percent (0-100), Audacity key "Duty Cycle".
    pub duty_cycle: f64,
    /// Linear amplitude.
    pub amplitude: f64,
    /// Output duration when generating into an empty buffer.
    pub duration: f64,
}
impl Dtmf {
    /// Replace the DTMF sequence (rejected if it contains invalid symbols, like Audacity).
    pub fn set_sequence(&mut self, sequence: impl Into<String>) -> bool {
        let s = sequence.into();
        if s.chars().all(|c| DTMF_SYMBOLS.contains(c)) { self.sequence = s; true } else { false }
    }
}
const DTMF_SYMBOLS: &str = "0123456789*#ABCDabcdefghijklmnopqrstuvwxyz";
/// Audacity's fade divisor: fs / 250 samples of linear fade in/out per tone chunk.
const DTMF_FADE: f64 = 250.0;
/// Audacity processes generators in blocks of the track's max block size (float: 1 MiB / 4).
pub const AUD_BLOCK: usize = 262_144;
fn dtmf_freqs(c: char) -> (f32, f32) {
    let f1 = match c { '1'|'2'|'3'|'A'|'a'..='f' => 697.0, '4'|'5'|'6'|'B'|'g'..='o' => 770.0,
        '7'|'8'|'9'|'C'|'p'..='z' => 852.0, '*'|'0'|'#'|'D' => 941.0, _ => 0.0 };
    let f2 = match c { '1'|'4'|'7'|'*'|'g'|'h'|'i'|'p'|'q'|'r'|'s' => 1209.0,
        '2'|'5'|'8'|'0'|'a'|'b'|'c'|'j'|'k'|'l'|'t'|'u'|'v' => 1336.0,
        '3'|'6'|'9'|'#'|'d'|'e'|'f'|'m'|'n'|'o'|'w'|'x'|'y'|'z' => 1477.0, 'A'|'B'|'C'|'D' => 1633.0, _ => 0.0 };
    (f1, f2)
}
/// `DtmfBase::MakeDtmfTone`: called per processing chunk; fades depend on the chunk length as in Audacity.
fn make_dtmf_tone(buf: &mut [f32], fs: f32, tone: char, last: u64, total: u64, amplitude: f32) {
    let (f1, f2) = dtmf_freqs(tone);
    let base = 2.0 * std::f64::consts::PI / fs as f64;
    let (a, b) = (base * f1 as f64, base * f2 as f64);
    let len = buf.len();
    for (i, o) in buf.iter_mut().enumerate() {
        let n = (i as u64 + last) as f64;
        *o = (amplitude as f64 * 0.5 * ((a * n).sin() + (b * n).sin())) as f32;
    }
    let fade = (len as f64).min(fs as f64 / DTMF_FADE);
    if last == 0 {
        let mut i = 0usize;
        while (i as f64) < fade { buf[i] = (buf[i] as f64 * (i as f64 / fade)) as f32; i += 1; }
    }
    if last as i64 >= total as i64 - len as i64 {
        let offset = (len as f64 - fade) as usize; // C++: size_t offset = len - A (double, truncated)
        let mut i = 0usize;
        while (i as f64) < fade && i + offset < len { buf[i + offset] = (buf[i + offset] as f64 * (1.0 - i as f64 / fade)) as f32; i += 1; }
    }
}
/// Renders `n` samples exactly like `ProcessInitialize` + `ProcessBlock` called with `block`-sized buffers.
pub fn dtmf_render(seq: &str, duty: f64, amplitude: f64, fs: f64, n: usize, block: usize) -> Vec<f32> {
    let tones: Vec<char> = seq.chars().collect();
    let nt = tones.len() as i64;
    if nt == 0 { return vec![0.0; n]; }
    let duration = n as f64 / fs;
    let (tone_s, sil_s) = if nt == 1 { (duration, 0.0) } else {
        let slot = duration / (nt as f64 + duty / 100.0 - 1.0);
        (slot * (duty / 100.0), slot * (1.0 - duty / 100.0))
    };
    let seq_n = (duration * fs + 0.5).floor() as i64;
    let mut n_tone = (tone_s * fs).floor() as i64;
    let mut n_sil = (sil_s * fs).floor() as i64;
    let mut diff = seq_n - nt * n_tone - (nt - 1) * n_sil;
    while diff > 2 * nt - 1 {
        n_tone += diff / nt;
        n_sil += diff / (nt - 1);
        diff = seq_n - nt * n_tone - (nt - 1) * n_sil;
    }
    let mut out = vec![0.0f32; n];
    let (mut cur, mut is_tone, mut remaining, mut tone_pos) = (-1i64, false, 0i64, 0u64);
    let mut pos = 0usize;
    'outer: while pos < n {
        let mut size = block.min(n - pos);
        while size > 0 {
            if remaining == 0 {
                is_tone = !is_tone;
                if is_tone { cur += 1; remaining = n_tone; tone_pos = 0; } else { remaining = n_sil; }
                if diff > 0 { remaining += 1; }
                diff -= 1;
                if remaining == 0 { continue; }
            }
            let len = (size as i64).min(remaining) as usize;
            if is_tone {
                if cur >= nt { break 'outer; }
                let c = tones[cur as usize];
                make_dtmf_tone(&mut out[pos..pos + len], fs as f32, c, tone_pos, n_tone as u64, amplitude as f32);
                tone_pos += len as u64;
            }
            remaining -= len as i64; pos += len; size -= len;
        }
    }
    out
}
impl Default for Dtmf { fn default()->Self{Self{sequence:"audacity".into(),duty_cycle:55.0,amplitude:0.8,duration:30.0}} }
impl BuiltinEffect for Dtmf {
    fn id(&self)->&'static str{"DTMF Tones"}
    fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![ParamDef::float("Duty Cycle","Duty cycle (%)",0.0,100.0,55.0),ParamDef::float("Amplitude","Amplitude",0.001,1.0,0.8),ParamDef::float("Duration","Duration",0.0,3600.0,30.0)]}
    fn get(&self,k:&str)->Option<f64>{match k{"Duty Cycle"=>Some(self.duty_cycle),"Amplitude"=>Some(self.amplitude),"Duration"=>Some(self.duration),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{"Duty Cycle"=>self.duty_cycle=v,"Amplitude"=>self.amplitude=v,"Duration"=>self.duration=v,_=>return false}true}
    fn text_keys(&self)->Vec<&'static str>{vec!["Sequence"]}
    fn get_text(&self,k:&str)->Option<String>{(k=="Sequence").then(||self.sequence.clone())}
    fn set_text(&mut self,k:&str,v:&str)->bool{k=="Sequence"&&self.set_sequence(v)}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{
        if self.sequence.is_empty(){return Err(EffectError::Invalid("DTMF sequence empty.".into()));}
        let n=duration_frames(b,self.duration);
        let out=dtmf_render(&self.sequence,self.duty_cycle,self.amplitude,b.sample_rate.max(1) as f64,n,AUD_BLOCK);
        resize_generated(b,n,&out);Ok(())
    }
}

/// Generate silence.
#[derive(Clone, Debug)]
pub struct Silence { 
    /// Output duration in seconds when generating into an empty buffer.
    pub duration:f64
}
impl Default for Silence{fn default()->Self{Self{duration:30.0}}}
impl BuiltinEffect for Silence{
    fn id(&self)->&'static str{"Silence"} fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![ParamDef::float("Duration","Duration",0.0,3600.0,30.0)]}
    fn get(&self,k:&str)->Option<f64>{if k=="Duration"{Some(self.duration)}else{None}}
    fn set(&mut self,k:&str,v:f64)->bool{if k=="Duration"{self.duration=v;true}else{false}}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{let n=duration_frames(b,self.duration);for c in &mut b.channels{c.resize(n,0.0);c.fill(0.0)}Ok(())}
}

/// Generate a rhythm track with simple synthesized clicks.
#[derive(Clone, Debug)]
pub struct RhythmTrack {
    /// Tempo in beats per minute.
    pub tempo:f64, /// Beats per bar.
    pub timesig:usize, /// Swing amount.
    pub swing:f64, /// Number of bars; zero uses click-track duration.
    pub bars:usize, /// Track duration in seconds.
    pub click_track_dur:f64, /// Leading silence in seconds.
    pub offset:f64, /// Click-type index.
    pub click_type:usize, /// Strong-beat MIDI pitch.
    pub high:usize, /// Weak-beat MIDI pitch.
    pub low:usize, /// Output duration when generating into an empty buffer.
    pub duration:f64,
}
impl Default for RhythmTrack{fn default()->Self{Self{tempo:120.0,timesig:4,swing:0.0,bars:16,click_track_dur:0.0,offset:0.0,click_type:0,high:84,low:80,duration:30.0}}}
impl BuiltinEffect for RhythmTrack{
    fn id(&self)->&'static str{"RhythmTrack"} fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![ParamDef::float("TEMPO","Tempo",30.0,300.0,120.0),ParamDef::int("TIMESIG","Beats per bar",1.0,20.0,4.0),ParamDef::float("SWING","Swing",-1.0,1.0,0.0),ParamDef::int("BARS","Number of bars",0.0,1000.0,16.0),ParamDef::float("CLICK-TRACK-DUR","Rhythm track duration",0.0,3600.0,0.0),ParamDef::float("OFFSET","Start time offset",0.0,3600.0,0.0),ParamDef::choice("CLICK-TYPE","Beat sound",&["Metronome","Ping (short)","Ping (long)","Cowbell","Resonant noise","Noise click","Drip (short)","Drip (long)"],0),ParamDef::int("HIGH","Strong beat MIDI pitch",18.0,116.0,84.0),ParamDef::int("LOW","Weak beat MIDI pitch",18.0,116.0,80.0),ParamDef::float("Duration","Duration",0.0,3600.0,30.0)]}
    fn get(&self,k:&str)->Option<f64>{match k{"TEMPO"=>Some(self.tempo),"TIMESIG"=>Some(self.timesig as f64),"SWING"=>Some(self.swing),"BARS"=>Some(self.bars as f64),"CLICK-TRACK-DUR"=>Some(self.click_track_dur),"OFFSET"=>Some(self.offset),"CLICK-TYPE"=>Some(self.click_type as f64),"HIGH"=>Some(self.high as f64),"LOW"=>Some(self.low as f64),"Duration"=>Some(self.duration),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{"TEMPO"=>self.tempo=v,"TIMESIG"=>self.timesig=v as usize,"SWING"=>self.swing=v,"BARS"=>self.bars=v as usize,"CLICK-TRACK-DUR"=>self.click_track_dur=v,"OFFSET"=>self.offset=v,"CLICK-TYPE"=>self.click_type=v as usize,"HIGH"=>self.high=v as usize,"LOW"=>self.low=v as usize,"Duration"=>self.duration=v,_=>return false}true}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{let sr=b.sample_rate as f64;let beat=60.0/self.tempo.max(1.0);let bars=if self.bars==0{(self.click_track_dur/(beat*self.timesig.max(1) as f64)).ceil() as usize}else{self.bars};let n=duration_frames(b,self.duration).max(((self.offset+beat*self.timesig as f64*bars as f64)*sr).ceil() as usize);let mut out=vec![0.0;n];for bar in 0..bars{for beat_i in 0..self.timesig{let t=self.offset+(bar*self.timesig+beat_i) as f64*beat+if beat_i%2==1{self.swing*beat/3.0}else{0.0};let start=(t*sr) as usize;let freq=440.0*2f64.powf((if beat_i==0{self.high}else{self.low} as f64-69.0)/12.0);for j in 0..(sr*0.08) as usize{if start+j>=n{break}let env=(-(j as f64)/(sr*0.018)).exp();out[start+j]+=(0.7*env*(2.0*PI*freq*j as f64/sr).sin()) as f32;}}}resize_generated(b,n,&out);Ok(())}
}

/// Generate a decaying plucked-string sound.
#[derive(Clone, Debug)]
pub struct Pluck { 
    /// MIDI pitch.
    pub pitch:usize,
    /// Fade mode: abrupt or gradual.
    pub fade:usize,
    /// Duration in seconds.
    pub dur:f64
}
impl Default for Pluck{fn default()->Self{Self{pitch:60,fade:0,dur:1.0}}}
impl BuiltinEffect for Pluck{
    fn id(&self)->&'static str{"Pluck"} fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![ParamDef::int("PITCH","Pluck MIDI pitch",1.0,120.0,60.0),ParamDef::choice("FADE","Fade-out type",&["Abrupt","Gradual"],0),ParamDef::float("DUR","Duration",0.0,60.0,1.0),ParamDef::float("Duration","Duration",0.0,60.0,1.0)]}
    fn get(&self,k:&str)->Option<f64>{match k{"PITCH"=>Some(self.pitch as f64),"FADE"=>Some(self.fade as f64),"DUR"|"Duration"=>Some(self.dur),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{"PITCH"=>self.pitch=v as usize,"FADE"=>self.fade=v as usize,"DUR"|"Duration"=>self.dur=v,_=>return false}true}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{// pluck.ny always returns DUR seconds (the selection is replaced).
        let n=(self.dur*b.sample_rate as f64).round() as usize;let sr=b.sample_rate.max(1) as f64;let hz=440.0*2f64.powf((self.pitch as f64-69.0)/12.0);let delay=(sr/hz).max(2.0) as usize;let mut ring=vec![0.0f64;delay];let mut seed=1234567u64;for x in &mut ring{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);*x=((seed>>33) as f64/2147483648.0)*2.0-1.0}let mut out=Vec::with_capacity(n);for i in 0..n{let x=ring[i%delay];let next=0.5*(x+ring[(i+1)%delay])*0.996;ring[i%delay]=next;let env=if self.fade==0{if i<n{1.0}else{0.0}}else{(1.0-0.999*(i as f64/n.max(1) as f64)).max(0.001)};out.push((x*env) as f32)}resize_generated(b,n,&out);Ok(())}
}

/// Generate a Risset-style drum.
#[derive(Clone, Debug)]
pub struct RissetDrum { 
    /// Fundamental frequency in Hz.
    pub freq:f64,
    /// Decay in seconds.
    pub decay:f64,
    /// Noise center frequency in Hz.
    pub cf:f64,
    /// Noise bandwidth in Hz.
    pub bw:f64,
    /// Noise mix percentage.
    pub noise:f64,
    /// Output amplitude.
    pub gain:f64
}
impl Default for RissetDrum{fn default()->Self{Self{freq:100.0,decay:2.0,cf:500.0,bw:400.0,noise:25.0,gain:0.8}}}
impl BuiltinEffect for RissetDrum{
    fn id(&self)->&'static str{"RissetDrum"} fn kind(&self)->EffectKind{EffectKind::Generate}
    fn params(&self)->Vec<ParamDef>{vec![ParamDef::float("FREQ","Frequency",50.0,2000.0,100.0),ParamDef::float("DECAY","Decay",0.1,60.0,2.0),ParamDef::float("CF","Center frequency",100.0,5000.0,500.0),ParamDef::float("BW","Width",10.0,1000.0,400.0),ParamDef::float("NOISE","Noise amount",0.0,100.0,25.0),ParamDef::float("GAIN","Amplitude",0.0,1.0,0.8),ParamDef::float("Duration","Duration",0.0,3600.0,30.0)]}
    fn get(&self,k:&str)->Option<f64>{match k{"FREQ"=>Some(self.freq),"DECAY"=>Some(self.decay),"CF"=>Some(self.cf),"BW"=>Some(self.bw),"NOISE"=>Some(self.noise),"GAIN"=>Some(self.gain),"Duration"=>Some(30.0),_=>None}}
    fn set(&mut self,k:&str,v:f64)->bool{match k{"FREQ"=>self.freq=v,"DECAY"=>self.decay=v,"CF"=>self.cf=v,"BW"=>self.bw=v,"NOISE"=>self.noise=v,"GAIN"=>self.gain=v,"Duration"=>{},_=>return false}true}
    fn process(&mut self,b:&mut AudioBuffer,_:&mut EffectContext)->Result<(),EffectError>{let n=duration_frames(b,self.decay);let sr=b.sample_rate.max(1) as f64;let mix=self.noise/100.0;let mut seed=7u64;let mut out=Vec::with_capacity(n);for i in 0..n{let t=i as f64/sr;seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);let w=((seed>>32) as f64/u32::MAX as f64)*2.0-1.0;let env=(-t/(self.decay*0.12).max(0.001)).exp();let tone=(1.0-mix)*(2.0*PI*self.freq*t).sin()*(-t/self.decay.max(0.001)).exp();let noise=mix*w*(2.0*PI*self.cf*t).sin()*(-t/(self.decay*0.25).max(0.001)).exp();out.push(((tone+noise)*env*self.gain) as f32)}resize_generated(b,n,&out);Ok(())}
}

/// Returns one default instance of each generator in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Tone::default()),Box::new(Chirp::default()),Box::new(Noise::default()),
        Box::new(Dtmf::default()),Box::new(Silence::default()),Box::new(RhythmTrack::default()),
        Box::new(Pluck::default()),Box::new(RissetDrum::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generators_smoke_repeatable_empty_and_clamping() {
        for mut effect in all() {
            let mut a=AudioBuffer::from_channels(8000,vec![vec![0.0;512],vec![0.0;512]]);
            let mut b=a.clone();
            let mut ctx=EffectContext::default();
            effect.process(&mut a,&mut ctx).unwrap();
            effect.process(&mut b,&mut ctx).unwrap();
            assert_eq!(a.channels,b.channels);
            assert_eq!(a.channels.len(),2);
            let mut empty=AudioBuffer::from_channels(8000,vec![vec![],vec![]]);
            effect.process(&mut empty,&mut ctx).unwrap();
            if let Some(p)=effect.params().first(){set_param(effect.as_mut(),p.key,f64::INFINITY);}
        }
    }
    #[test]
    fn tone_and_silence_sanity() {
        let mut tone=Tone::default();
        let mut b=AudioBuffer::from_channels(8000,vec![vec![0.0;32]]);
        tone.process(&mut b,&mut EffectContext::default()).unwrap();
        assert!(b.channels[0].iter().any(|x|x.abs()>0.1));
        let mut silence=Silence::default();
        silence.process(&mut b,&mut EffectContext::default()).unwrap();
        assert!(b.channels[0].iter().all(|x|*x==0.0));
    }
}
