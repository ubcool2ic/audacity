//! Track drop-down menu operations (WaveTrackControls.cpp, Audacity 3.7.3).
use crate::timeline::WaveClips;
use crate::{AudioBuffer, Track};

fn map_buffers(clips: &mut WaveClips, f: impl Fn(&mut AudioBuffer)) {
    for clip in &mut clips.clips {
        f(&mut clip.buffer);
        if let Some(tl) = &mut clip.trim_left {
            f(tl);
        }
        if let Some(tr) = &mut clip.trim_right {
            f(tr);
        }
    }
    if let Some(hh) = &mut clips.hidden_head {
        f(&mut hh.audio);
    }
}

/// Swaps channel 0 and channel 1 in every buffer if the track is stereo.
pub fn swap_channels(t: &mut Track) {
    if t.clips.n_channels == 2 {
        map_buffers(&mut t.clips, |buf| {
            if buf.channels.len() >= 2 {
                buf.channels.swap(0, 1);
            }
        });
    }
}

/// Splits each channel of `t` into an independent mono track.
pub fn split_channels(t: &Track, to_mono: bool) -> Vec<Track> {
    let n = t.clips.n_channels;
    let mut result = Vec::with_capacity(n);

    for ch in 0..n {
        let mut track = t.clone();
        track.clips.n_channels = 1;
        track.clips.cut_lines.clear();

        map_buffers(&mut track.clips, |buf| {
            if ch < buf.channels.len() {
                buf.channels = vec![buf.channels[ch].clone()];
            } else {
                buf.channels = vec![Vec::new()];
            }
        });

        if to_mono {
            track.pan = 0.0;
        } else {
            track.pan = if ch == 0 { -1.0 } else { 1.0 };
        }

        result.push(track);
    }

    result
}

/// Combines two mono tracks into a stereo track if compatible.
pub fn make_stereo(left: &Track, right: &Track) -> Option<Track> {
    if left.clips.n_channels != 1 || right.clips.n_channels != 1 {
        return None;
    }
    if left.sample_rate() != right.sample_rate() {
        return None;
    }
    if left.clips.clips.len() != right.clips.clips.len() {
        return None;
    }
    if left.clips.hidden_head.is_some() || right.clips.hidden_head.is_some() {
        return None;
    }

    for (lc, rc) in left.clips.clips.iter().zip(&right.clips.clips) {
        if lc.start != rc.start {
            return None;
        }
        if lc.buffer.channels.first().map_or(0, |v| v.len())
            != rc.buffer.channels.first().map_or(0, |v| v.len())
        {
            return None;
        }
        let lc_tl_len = lc.trim_left.as_ref().and_then(|b| b.channels.first()).map_or(0, |v| v.len());
        let rc_tl_len = rc.trim_left.as_ref().and_then(|b| b.channels.first()).map_or(0, |v| v.len());
        if lc_tl_len != rc_tl_len {
            return None;
        }
        let lc_tr_len = lc.trim_right.as_ref().and_then(|b| b.channels.first()).map_or(0, |v| v.len());
        let rc_tr_len = rc.trim_right.as_ref().and_then(|b| b.channels.first()).map_or(0, |v| v.len());
        if lc_tr_len != rc_tr_len {
            return None;
        }
    }

    let mut res = left.clone();
    res.clips.n_channels = 2;
    res.pan = 0.0;
    res.gain = left.gain;

    for (i, clip) in res.clips.clips.iter_mut().enumerate() {
        let r_clip = &right.clips.clips[i];
        if let Some(r_ch) = r_clip.buffer.channels.first() {
            clip.buffer.channels.push(r_ch.clone());
        }
        if let (Some(l_tl), Some(r_tl)) = (&mut clip.trim_left, &r_clip.trim_left) {
            if let Some(r_ch) = r_tl.channels.first() {
                l_tl.channels.push(r_ch.clone());
            }
        }
        if let (Some(l_tr), Some(r_tr)) = (&mut clip.trim_right, &r_clip.trim_right) {
            if let Some(r_ch) = r_tr.channels.first() {
                l_tr.channels.push(r_ch.clone());
            }
        }
    }

    Some(res)
}

/// Changes track sample rate while keeping clip start times.
pub fn set_rate(t: &mut Track, rate: u32) {
    if rate == 0 || rate == t.clips.sample_rate {
        return;
    }
    let old_rate = t.clips.sample_rate as f64;
    let new_rate = rate as f64;

    for clip in &mut t.clips.clips {
        let new_start = ((clip.start as f64) * new_rate / old_rate).round() as usize;
        clip.start = new_start;
    }

    t.clips.sample_rate = rate;
    map_buffers(&mut t.clips, |buf| {
        buf.sample_rate = rate;
    });
}

/// Sets sample integer format and quantizes track if specified.
pub fn set_format(t: &mut Track, int_bits: Option<u32>) {
    t.int_bits = int_bits;
    if int_bits.is_some() {
        t.quantize_range(0, usize::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioBuffer, Track};

    #[test]
    fn test_swap_channels() {
        let ch0 = vec![1.0, 2.0, 3.0];
        let ch1 = vec![4.0, 5.0, 6.0];
        let buf = AudioBuffer::from_channels(44100, vec![ch0.clone(), ch1.clone()]);
        let mut track = Track::new("Stereo", buf);
        swap_channels(&mut track);
        assert_eq!(track.clips.clips[0].buffer.channels[0], ch1);
        assert_eq!(track.clips.clips[0].buffer.channels[1], ch0);
    }

    #[test]
    fn test_split_stereo_pans() {
        let buf = AudioBuffer::from_channels(44100, vec![vec![1.0], vec![2.0]]);
        let track = Track::new("Stereo", buf);

        let split = split_channels(&track, false);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].pan, -1.0);
        assert_eq!(split[1].pan, 1.0);

        let split_mono = split_channels(&track, true);
        assert_eq!(split_mono.len(), 2);
        assert_eq!(split_mono[0].pan, 0.0);
        assert_eq!(split_mono[1].pan, 0.0);
    }

    #[test]
    fn test_make_stereo_round_trip() {
        let ch0 = vec![0.1, 0.2, 0.3];
        let ch1 = vec![0.4, 0.5, 0.6];
        let buf = AudioBuffer::from_channels(44100, vec![ch0.clone(), ch1.clone()]);
        let track = Track::new("Stereo", buf);

        let split = split_channels(&track, true);
        let rejoined = make_stereo(&split[0], &split[1]).expect("make_stereo failed");
        assert_eq!(rejoined.clips.n_channels, 2);
        assert_eq!(rejoined.clips.clips[0].buffer.channels[0], ch0);
        assert_eq!(rejoined.clips.clips[0].buffer.channels[1], ch1);
    }

    #[test]
    fn test_set_rate_keeps_start_time() {
        let buf = AudioBuffer::from_channels(44100, vec![vec![0.0; 100]]);
        let mut track = Track::new("Track", buf);
        track.clips.clips[0].start = 44100;

        set_rate(&mut track, 22050);
        assert_eq!(track.clips.sample_rate, 22050);
        assert_eq!(track.clips.clips[0].start, 22050);
        assert_eq!(track.clips.clips[0].buffer.sample_rate, 22050);
    }
}
