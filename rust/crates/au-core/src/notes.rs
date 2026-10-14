use crate::AudioBuffer;

/// A note in an Audacity note track.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// Start time in seconds, relative to the track origin.
    pub start: f64,
    /// Duration in seconds.
    pub duration: f64,
    /// MIDI key number, including any fractional pitch.
    pub pitch: f64,
    /// Allegro loudness, conventionally MIDI velocity from 0 to 127.
    pub loudness: f64,
    /// Allegro voice/channel. A value of `-1` applies to every channel.
    pub channel: i32,
}

/// A note track with Allegro notes and Audacity track settings.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteTrack {
    /// Track name.
    pub name: String,
    /// Notes sorted by start time.
    pub notes: Vec<Note>,
    /// Track origin in seconds.
    pub offset: f64,
    /// Bitmask of visible MIDI channels.
    pub visible_channels: u32,
    /// Track gain offset in decibels.
    pub velocity: f32,
    /// Whether this track is muted.
    pub mute: bool,
    /// Whether this track is soloed.
    pub solo: bool,
    /// First tempo in beats per minute.
    pub tempo_bpm: f64,
    /// Allegro lines not otherwise represented by this type.
    pub extra_lines: Vec<String>,
}

impl NoteTrack {
    /// Creates an empty note track with Audacity's default track settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            notes: Vec::new(),
            offset: 0.0,
            visible_channels: 0xFFFF,
            velocity: 0.0,
            mute: false,
            solo: false,
            tempo_bpm: 120.0,
            extra_lines: Vec::new(),
        }
    }

    /// Returns the track offset plus the greatest note end time, or the offset
    /// if the track contains no notes.
    pub fn end_time(&self) -> f64 {
        self.notes
            .iter()
            .map(|note| note.start + note.duration)
            .fold(self.offset, |end, note_end| end.max(self.offset + note_end))
    }

    /// Returns the minimum and maximum note pitches, or `None` for an empty
    /// track.
    pub fn pitch_range(&self) -> Option<(f64, f64)> {
        let mut pitches = self.notes.iter().map(|note| note.pitch);
        let first = pitches.next()?;
        Some(pitches.fold((first, first), |(min, max), pitch| {
            (min.min(pitch), max.max(pitch))
        }))
    }

    /// Sorts notes by start time.
    pub fn sort(&mut self) {
        self.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    }

    /// Serializes this track as Allegro text in seconds, with the track origin
    /// kept separately in Audacity's `offset` attribute.
    pub fn to_allegro(&self) -> String {
        let mut output = String::new();
        output.push_str("#offset 0\n");
        // Audacity keeps the track name in the XML, so the sequence track is unnamed.
        output.push_str("#track 0\n");
        output.push_str("T");
        output.push_str(&format_time(0.0));
        output.push_str(" -tempor:");
        output.push_str(&format_g6(self.tempo_bpm));
        output.push('\n');

        for line in &self.extra_lines {
            output.push_str(line);
            if !line.ends_with('\n') {
                output.push('\n');
            }
        }

        let mut notes = self.notes.iter().collect::<Vec<_>>();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));

        for note in notes {
            output.push('T');
            output.push_str(&format_time(note.start));
            output.push_str(" V");
            if note.channel == -1 {
                output.push('-');
            } else {
                output.push_str(&note.channel.to_string());
            }
            output.push_str(" K");
            output.push_str(&note.pitch.round().to_string());
            output.push_str(" P");
            output.push_str(&format_g6(note.pitch));
            output.push_str(" U");
            output.push_str(&format_time(note.duration));
            output.push_str(" L");
            output.push_str(&format_g6(note.loudness));
            output.push('\n');
        }

        output
    }

    /// Parses Allegro text, collecting notes from all tracks.
    ///
    /// Seconds-based `T` times and durations are supported alongside `TW`
    /// whole-note and `TB` beat times. Musical duration symbols are interpreted
    /// using the tempo map.
    pub fn from_allegro(text: &str) -> Result<NoteTrack, String> {
        let mut track = NoteTrack::default();
        let mut raw_tempos = Vec::<RawTempo>::new();
        let mut saw_zero_first_tempo = false;

        let mut channel = 0_i32;
        let mut key = 60_i32;
        let mut loudness = 100.0_f64;
        let mut pitch = 60.0_f64;
        let mut duration = DurationSpec::Seconds(1.0);
        let mut current_seconds = 0.0_f64;
        let mut current_beat = 0.0_f64;
        let mut current_mode = TimeMode::Seconds;

        for (line_index, original_line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            let content = strip_comment(original_line).trim();
            if content.is_empty() {
                if original_line.trim_start().starts_with(';') {
                    track.extra_lines.push(original_line.to_string());
                }
                continue;
            }

            if let Some(rest) = content.strip_prefix("#offset") {
                let value = rest.trim();
                track.offset = value
                    .parse::<f64>()
                    .map_err(|_| format!("invalid #offset on line {line_number}"))?;
                continue;
            }

            if let Some(rest) = content.strip_prefix("#track") {
                let mut parts = rest.trim_start().splitn(2, char::is_whitespace);
                let number = parts
                    .next()
                    .unwrap_or_default()
                    .parse::<i32>()
                    .map_err(|_| format!("invalid #track on line {line_number}"))?;
                let name = parts.next().unwrap_or_default().trim();
                if number == 0 && !name.is_empty() {
                    track.name = name.to_string();
                } else if number != 0 {
                    track.extra_lines.push(original_line.to_string());
                }
                continue;
            }

            let tokens = tokenize(content);
            if tokens.is_empty() {
                continue;
            }

            let mut time_token = None::<(TimeMode, f64)>;
            let mut tempo_token = None::<f64>;
            let mut is_timesig = false;

            for token in &tokens {
                if let Some((mode, value)) = parse_time_token(token) {
                    time_token = Some((mode, value));
                }
                if let Some(value) = token.strip_prefix("-tempor:") {
                    if let Ok(bpm) = value.parse::<f64>() {
                        tempo_token = Some(bpm);
                    }
                }
                if token.starts_with("-timesig_numr:") || token.starts_with("-timesig_denr:") {
                    is_timesig = true;
                }
            }

            if let Some(bpm) = tempo_token {
                let (mode, value) = time_token.unwrap_or((TimeMode::Seconds, 0.0));
                let is_zero = value == 0.0;
                if raw_tempos.is_empty() {
                    track.tempo_bpm = bpm;
                    saw_zero_first_tempo = is_zero;
                }
                raw_tempos.push(RawTempo {
                    mode,
                    value,
                    bpm,
                    line: original_line.to_string(),
                });
                continue;
            }

            if is_timesig {
                track.extra_lines.push(original_line.to_string());
                continue;
            }

            let mut new_key = None::<i32>;
            let mut new_pitch = None::<f64>;
            let mut new_loudness = None::<f64>;
            let mut new_duration = None::<DurationSpec>;
            let mut next_duration = None::<DurationSpec>;
            let mut has_voice = false;
            let mut has_time = false;
            let mut has_note_field = false;

            for token in &tokens {
                if let Some((mode, value)) = parse_time_token(token) {
                    if has_time {
                        return Err(format!("time specified twice on line {line_number}"));
                    }
                    has_time = true;
                    current_mode = mode;
                    match mode {
                        TimeMode::Seconds => current_seconds = value,
                        TimeMode::Beats => current_beat = value,
                    }
                    continue;
                }

                let Some(first) = token.chars().next() else {
                    continue;
                };
                let upper = first.to_ascii_uppercase();

                match upper {
                    'V' => {
                        if has_voice {
                            return Err(format!("voice specified twice on line {line_number}"));
                        }
                        has_voice = true;
                        let value = &token[1..];
                        channel = if value == "-" {
                            -1
                        } else {
                            value.parse::<i32>().map_err(|_| {
                                format!("invalid channel on line {line_number}")
                            })?
                        };
                    }
                    'K' => {
                        has_note_field = true;
                        new_key = Some(parse_key(&token[1..]).ok_or_else(|| {
                            format!("invalid key on line {line_number}")
                        })?);
                    }
                    'P' => {
                        has_note_field = true;
                        new_pitch = Some(parse_pitch(&token[1..]).ok_or_else(|| {
                            format!("invalid pitch on line {line_number}")
                        })?);
                    }
                    'L' => {
                        has_note_field = true;
                        new_loudness = Some(parse_loudness(&token[1..]).ok_or_else(|| {
                            format!("invalid loudness on line {line_number}")
                        })?);
                    }
                    'U' | 'Q' => {
                        has_note_field = true;
                        new_duration = Some(parse_duration_token(token).ok_or_else(|| {
                            format!("invalid duration on line {line_number}")
                        })?);
                    }
                    'S' | 'I' | 'H' | 'W' => {
                        has_note_field = true;
                        new_duration = Some(parse_duration_token(token).ok_or_else(|| {
                            format!("invalid duration on line {line_number}")
                        })?);
                    }
                    'N' => {
                        next_duration = Some(parse_duration_token(token).ok_or_else(|| {
                            format!("invalid next duration on line {line_number}")
                        })?);
                    }
                    '-' => {
                        // Allegro attributes not represented by this module are
                        // intentionally ignored, as in the reader.
                    }
                    _ => {}
                }
            }

            if let Some(value) = new_key {
                key = value;
            }
            if let Some(value) = new_pitch {
                pitch = value;
                if new_key.is_none() {
                    key = (value + 0.5).floor() as i32;
                }
            } else if new_key.is_some() && key < 128 {
                pitch = key as f64;
            }

            if let Some(value) = new_loudness {
                loudness = value;
            }
            if let Some(value) = new_duration {
                duration = value;
            }

            let is_note = new_pitch.is_some()
                || new_duration.is_some()
                || (new_key.is_some() && key < 128);

            if !is_note {
                track.extra_lines.push(original_line.to_string());
                continue;
            }

            let start_seconds = match current_mode {
                TimeMode::Seconds => current_seconds,
                TimeMode::Beats => {
                    let initial = if track.tempo_bpm.is_finite() && track.tempo_bpm > 0.0 {
                        track.tempo_bpm
                    } else {
                        120.0
                    };
                    let changes: Vec<TempoChange> = raw_tempos
                        .iter()
                        .filter(|t| matches!(t.mode, TimeMode::Beats))
                        .map(|t| TempoChange { beat: t.value, bpm: t.bpm })
                        .collect();
                    beats_to_seconds(current_beat, initial, &changes)
                }
            };

            track.notes.push(Note {
                start: start_seconds,
                duration: duration_to_seconds(duration, start_seconds, track.tempo_bpm, &raw_tempos),
                pitch,
                loudness,
                channel,
            });

            let advance = next_duration.unwrap_or(duration);
            let advance_seconds =
                duration_to_seconds(advance, start_seconds, track.tempo_bpm, &raw_tempos);
            current_seconds = start_seconds + advance_seconds;
            current_mode = TimeMode::Seconds;
            if !has_note_field {
                current_beat = current_seconds * track.tempo_bpm / 60.0;
            }
        }

        let mut tempo_changes = Vec::<TempoChange>::new();
        let mut initial_tempo = 120.0;
        for (index, raw) in raw_tempos.iter().enumerate() {
            if index == 0 && raw.bpm.is_finite() && raw.bpm > 0.0 {
                initial_tempo = raw.bpm;
            }
            let beat = match raw.mode {
                TimeMode::Beats => raw.value,
                TimeMode::Seconds => {
                    seconds_to_beats(raw.value, initial_tempo, &tempo_changes)
                }
            };
            tempo_changes.push(TempoChange {
                beat,
                bpm: raw.bpm,
            });
            tempo_changes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        }

        if !raw_tempos.is_empty() {
            track.tempo_bpm = raw_tempos[0].bpm;
        }

        for (index, raw) in raw_tempos.iter().enumerate() {
            let is_initial_zero = index == 0
                && saw_zero_first_tempo
                && raw.value == 0.0;
            if !is_initial_zero {
                track.extra_lines.push(raw.line.clone());
            }
        }

        // Recalculate musical-time starts and durations using the completed
        // tempo map where possible.
        track.sort();
        Ok(track)
    }

    /// Renders this track as a mono built-in synthesizer buffer.
    ///
    /// The sound consists of a sine wave and its second and third harmonics,
    /// with a short attack, release, track gain, and soft clipping.
    pub fn render(&self, sample_rate: u32) -> AudioBuffer {
        let render_end = self.end_time() + 0.1;
        let frame_count = if sample_rate == 0 || !render_end.is_finite() || render_end <= 0.0 {
            0
        } else {
            (render_end * f64::from(sample_rate)).ceil() as usize
        };
        let mut samples = vec![0.0_f32; frame_count];

        if !self.mute && sample_rate != 0 {
            let gain = 10.0_f64.powf(f64::from(self.velocity) / 20.0);
            let sample_rate_f64 = f64::from(sample_rate);

            for note in &self.notes {
                if !channel_is_visible(note.channel, self.visible_channels)
                    || !note.start.is_finite()
                    || !note.duration.is_finite()
                    || note.duration <= 0.0
                    || !note.pitch.is_finite()
                {
                    continue;
                }

                let start = self.offset + note.start;
                let note_end = start + note.duration;
                let amplitude = (note.loudness / 127.0) * gain * 0.2;
                let frequency = 440.0 * 2.0_f64.powf((note.pitch - 69.0) / 12.0);

                for (index, sample) in samples.iter_mut().enumerate() {
                    let time = index as f64 / sample_rate_f64;
                    if time < start || time >= note_end + 0.05 {
                        continue;
                    }

                    let attack = ((time - start) / 0.005).clamp(0.0, 1.0);
                    let release = if time > note_end {
                        ((note_end + 0.05 - time) / 0.05).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let envelope = attack * release;
                    let phase = std::f64::consts::TAU * frequency * (time - start);
                    let wave = phase.sin()
                        + 0.5 * (2.0 * phase).sin()
                        + 0.25 * (3.0 * phase).sin();
                    *sample += (amplitude * envelope * wave) as f32;
                }
            }

            for sample in &mut samples {
                *sample = f64::from(*sample).tanh() as f32;
            }
        }

        AudioBuffer::from_channels(sample_rate, vec![samples])
    }
}

impl Default for NoteTrack {
    fn default() -> Self {
        Self::new("Note Track")
    }
}

#[derive(Clone, Copy)]
enum TimeMode {
    Seconds,
    Beats,
}

#[derive(Clone, Copy)]
enum DurationSpec {
    Seconds(f64),
    Beats(f64),
}

struct RawTempo {
    mode: TimeMode,
    value: f64,
    bpm: f64,
    line: String,
}

#[derive(Clone, Copy)]
struct TempoChange {
    beat: f64,
    bpm: f64,
}

fn format_time(value: f64) -> String {
    format!("{value:.4}")
}

fn format_g6(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return value.to_string();
    }

    let exponent = value.abs().log10().floor() as i32;
    if exponent < -4 || exponent >= 6 {
        let scientific = format!("{:.5e}", value);
        let Some((mantissa, exponent_text)) = scientific.split_once('e') else {
            return scientific;
        };
        let mut mantissa = mantissa.to_string();
        while mantissa.contains('.') && mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
        let parsed_exponent = exponent_text.parse::<i32>().unwrap_or(0);
        let sign = if parsed_exponent >= 0 { '+' } else { '-' };
        return format!(
            "{mantissa}e{sign}{:02}",
            parsed_exponent.unsigned_abs()
        );
    }

    let decimals = (5 - exponent).max(0) as usize;
    let mut fixed = format!("{value:.decimals$}");
    while fixed.contains('.') && fixed.ends_with('0') {
        fixed.pop();
    }
    if fixed.ends_with('.') {
        fixed.pop();
    }
    fixed
}

fn channel_is_visible(channel: i32, mask: u32) -> bool {
    if channel < 0 {
        true
    } else if channel < 16 {
        mask & (1_u32 << channel) != 0
    } else {
        false
    }
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = None::<char>;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quoted.is_some() {
            escaped = true;
            continue;
        }
        if let Some(quote) = quoted {
            if character == quote {
                quoted = None;
            }
        } else if character == '"' || character == '\'' {
            quoted = Some(character);
        } else if character == ';' {
            return &line[..index];
        }
    }
    line
}

fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = None::<char>;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quoted.is_some() {
            current.push(character);
            escaped = true;
            continue;
        }
        if let Some(quote) = quoted {
            current.push(character);
            if character == quote {
                quoted = None;
            }
        } else if character == '"' || character == '\'' {
            quoted = Some(character);
            current.push(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_time_token(token: &str) -> Option<(TimeMode, f64)> {
    let upper = token.to_ascii_uppercase();
    let (mode, value) = if let Some(value) = upper.strip_prefix("TW") {
        (TimeMode::Beats, value.parse::<f64>().ok()? * 4.0)
    } else if let Some(value) = upper.strip_prefix("TB") {
        (TimeMode::Beats, value.parse::<f64>().ok()?)
    } else if let Some(value) = upper.strip_prefix('T') {
        (TimeMode::Seconds, value.parse::<f64>().ok()?)
    } else {
        return None;
    };
    Some((mode, value))
}

fn parse_pitch(value: &str) -> Option<f64> {
    if let Ok(number) = value.parse::<f64>() {
        return Some(number);
    }
    parse_key(value).map(|key| key as f64)
}

fn parse_key(value: &str) -> Option<i32> {
    if let Ok(number) = value.parse::<i32>() {
        return Some(number);
    }
    let mut characters = value.chars();
    let letter = characters.next()?.to_ascii_uppercase();
    let mut key = match letter {
        'A' => 21,
        'B' => 23,
        'C' => 12,
        'D' => 14,
        'E' => 16,
        'F' => 17,
        'G' => 19,
        _ => return None,
    };

    let remainder = characters.as_str();
    let octave_start = remainder
        .find(|character: char| character.is_ascii_digit() || character == '-')
        .unwrap_or(remainder.len());
    for accidental in remainder[..octave_start].chars() {
        match accidental.to_ascii_uppercase() {
            'S' => key += 1,
            'F' => key -= 1,
            _ => return None,
        }
    }
    let octave = &remainder[octave_start..];
    if !octave.is_empty() {
        key += octave.parse::<i32>().ok()? * 12;
    }
    Some(key)
}

fn parse_loudness(value: &str) -> Option<f64> {
    if let Ok(number) = value.parse::<f64>() {
        return Some(number);
    }
    match value.to_ascii_uppercase().as_str() {
        "FFF" => Some(127.0),
        "FF" => Some(120.0),
        "F" => Some(110.0),
        "MF" => Some(100.0),
        "MP" => Some(90.0),
        "P" => Some(80.0),
        "PP" => Some(70.0),
        "PPP" => Some(60.0),
        _ => None,
    }
}

fn parse_duration_token(token: &str) -> Option<DurationSpec> {
    let mut characters = token.chars();
    let first = characters.next()?.to_ascii_uppercase();
    let remainder = characters.as_str();

    if let Ok(seconds) = remainder.parse::<f64>() {
        return Some(DurationSpec::Seconds(seconds));
    }

    let base_beats = match first {
        'S' => 0.25,
        'I' => 0.5,
        'Q' => 1.0,
        'H' => 2.0,
        'W' => 4.0,
        'U' | 'N' => {
            return remainder.parse::<f64>().ok().map(DurationSpec::Seconds);
        }
        _ => return None,
    };

    if remainder.is_empty() {
        return Some(DurationSpec::Beats(base_beats));
    }

    let bytes = remainder.as_bytes();
    let mut position = 0;
    let mut beats = base_beats;
    let mut multiplier = 1.0;

    while position < bytes.len() {
        match bytes[position] as char {
            'T' | 't' => {
                beats *= 2.0 / 3.0;
                position += 1;
            }
            '.' => {
                beats *= 1.5;
                position += 1;
            }
            '+' => {
                // The reader supports additive durations. Preserve the leading
                // duration here; the additional component is not needed for
                // the common note-duration forms.
                break;
            }
            digit if digit.is_ascii_digit() || digit == '-' || digit == '.' => {
                let tail = &remainder[position..];
                let number_end = tail
                    .find(|character: char| {
                        !(character.is_ascii_digit()
                            || character == '.'
                            || character == '-')
                    })
                    .unwrap_or(tail.len());
                if number_end == 0 {
                    return None;
                }
                multiplier *= tail[..number_end].parse::<f64>().ok()?;
                position += number_end;
            }
            _ => return None,
        }
    }

    Some(DurationSpec::Beats(beats * multiplier))
}

fn duration_to_seconds(
    duration: DurationSpec,
    start_seconds: f64,
    initial_tempo: f64,
    tempos: &[RawTempo],
) -> f64 {
    match duration {
        DurationSpec::Seconds(seconds) => seconds,
        DurationSpec::Beats(beats) => {
            let changes = make_tempo_changes(tempos, initial_tempo);
            let start_beat = seconds_to_beats(start_seconds, initial_tempo, &changes);
            beats_to_seconds(start_beat + beats, initial_tempo, &tempos_to_changes(tempos, initial_tempo))
                - start_seconds
        }
    }
}

fn make_tempo_changes(tempos: &[RawTempo], initial_tempo: f64) -> Vec<TempoChange> {
    let mut changes = Vec::<TempoChange>::new();
    for raw in tempos {
        let beat = match raw.mode {
            TimeMode::Beats => raw.value,
            TimeMode::Seconds => seconds_to_beats(raw.value, initial_tempo, &changes),
        };
        changes.push(TempoChange {
            beat,
            bpm: raw.bpm,
        });
        changes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
    changes
}

fn tempos_to_changes(tempos: &[RawTempo], initial_tempo: f64) -> Vec<TempoChange> {
    make_tempo_changes(tempos, initial_tempo)
}

fn seconds_to_beats(seconds: f64, initial_tempo: f64, changes: &[TempoChange]) -> f64 {
    if !seconds.is_finite() {
        return seconds;
    }
    let mut low = seconds * initial_tempo / 60.0 - 100_000.0;
    let mut high = seconds * initial_tempo / 60.0 + 100_000.0;
    for _ in 0..80 {
        let middle = (low + high) * 0.5;
        if beats_to_seconds(middle, initial_tempo, changes) < seconds {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) * 0.5
}

fn beats_to_seconds(beat: f64, initial_tempo: f64, changes: &[TempoChange]) -> f64 {
    if !beat.is_finite() {
        return beat;
    }

    let mut points = changes
        .iter()
        .filter(|change| change.beat > 0.0 && change.beat < beat.max(0.0))
        .copied()
        .collect::<Vec<_>>();
    points.sort_by(|a, b| a.beat.total_cmp(&b.beat));

    let mut tempo = changes
        .iter()
        .filter(|change| change.beat <= 0.0)
        .max_by(|a, b| a.beat.total_cmp(&b.beat))
        .map(|change| change.bpm)
        .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
        .unwrap_or(initial_tempo);
    let mut cursor = 0.0;
    let mut seconds = 0.0;

    if beat >= 0.0 {
        for change in points {
            seconds += (change.beat - cursor) * 60.0 / tempo;
            cursor = change.beat;
            if change.bpm.is_finite() && change.bpm > 0.0 {
                tempo = change.bpm;
            }
        }
        seconds += (beat - cursor) * 60.0 / tempo;
        seconds
    } else {
        let mut boundaries = changes
            .iter()
            .filter(|change| change.beat > beat && change.beat < 0.0)
            .copied()
            .collect::<Vec<_>>();
        boundaries.sort_by(|a, b| b.beat.total_cmp(&a.beat));
        cursor = 0.0;
        for change in boundaries {
            let segment_tempo = changes
                .iter()
                .filter(|candidate| candidate.beat < cursor && candidate.beat >= change.beat)
                .max_by(|a, b| a.beat.total_cmp(&b.beat))
                .map(|candidate| candidate.bpm)
                .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
                .unwrap_or(initial_tempo);
            seconds -= (cursor - change.beat) * 60.0 / segment_tempo;
            cursor = change.beat;
        }
        let segment_tempo = changes
            .iter()
            .filter(|change| change.beat <= cursor && change.beat > beat)
            .max_by(|a, b| a.beat.total_cmp(&b.beat))
            .map(|change| change.bpm)
            .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
            .unwrap_or(initial_tempo);
        seconds -= (cursor - beat) * 60.0 / segment_tempo;
        seconds
    }
}
