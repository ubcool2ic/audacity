// SPDX-License-Identifier: GPL-3.0-or-later
//! Verification helper: apply built-in effects to WAVs and extract audio from .aup3.
//! fx_tool apply <id> in.wav out.wav [Key=value ...] [aux=path.wav] [profile=noise.wav]
//! fx_tool gen <id> out.wav <rate> <channels> <seconds> [Key=value ...]
//! fx_tool analyze <id> in.wav [Key=value ...]
//! fx_tool aup3wav in.aup3 out.wav [track]
use au_effects::core::*;
use au_effects::registry;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kv = |rest: &[String], e: &mut dyn BuiltinEffect| -> (Option<String>, Option<String>) {
        let (mut aux, mut prof) = (None, None);
        let mut strs = Vec::new();
        for s in rest {
            let (k, v) = s.split_once('=').expect("Key=value");
            match k { "aux" => aux = Some(v.to_string()), "profile" => prof = Some(v.to_string()), _ => strs.push((k.to_string(), v.to_string())) }
        }
        au_effects::stack::params_from_strings(e, &strs);
        (aux, prof)
    };
    match a[1].as_str() {
        "apply" | "analyze" => {
            let mut e = registry::create(&a[2]).expect("unknown effect");
            let mut buf = au_io::import_file(Path::new(&a[3])).unwrap();
            let rest = if a[1] == "apply" { &a[5..] } else { &a[4..] };
            let (aux, prof) = kv(rest, e.as_mut());
            let auxb = aux.map(|p| au_io::import_file(Path::new(&p)).unwrap());
            if let Some(p) = prof {
                let nb = au_io::import_file(Path::new(&p)).unwrap();
                e.learn_profile(&nb).unwrap();
            }
            let mut ctx = EffectContext { aux: auxb.as_ref(), ..Default::default() };
            e.process(&mut buf, &mut ctx).unwrap();
            if a[1] == "apply" { au_io::export_wav(Path::new(&a[4]), &buf, 32).unwrap(); }
            for l in &ctx.labels { println!("LABEL\t{:.6}\t{:.6}\t{}", l.t0, l.t1, l.text); }
            if !ctx.report.is_empty() { println!("{}", ctx.report); }
        }
        "gen" => {
            let mut e = registry::create(&a[2]).expect("unknown");
            let rate: u32 = a[4].parse().unwrap(); let ch: usize = a[5].parse().unwrap(); let secs: f64 = a[6].parse().unwrap();
            kv(&a[7..], e.as_mut());
            let n = (secs * rate as f64).round() as usize;
            let mut buf = au_core::AudioBuffer::from_channels(rate, vec![vec![0.0; n]; ch]);
            e.process(&mut buf, &mut EffectContext::default()).unwrap();
            au_io::export_wav(Path::new(&a[3]), &buf, 32).unwrap();
        }
        "mkaup3" => {
            // mkaup3 in.wav out.aup3 [tags.txt with NAME=value lines]
            let buf = au_io::import_file(Path::new(&a[2])).unwrap();
            let rate = buf.sample_rate;
            let mut p = au_project::aup3::Aup3Project {
                state: au_core::ProjectState { tracks: vec![au_core::Track::new("in", buf)], sample_rate: rate, labels: vec![], sync_lock: false, note_tracks: Default::default() },
                extras: Default::default(),
            };
            p.extras.track_formats = vec![au_project::blocks::SampleFormat::Float];
            p.extras.track_selected = vec![true];
            p.extras.user_version = 0x0307_0000; // avoid the 3.7 "project update required" dialog
            if let Some(tf) = a.get(4) {
                for line in std::fs::read_to_string(tf).unwrap().lines() {
                    if let Some((k, v)) = line.split_once('=') { p.extras.tags.set(k, v); }
                }
            }
            let _ = std::fs::remove_file(&a[3]);
            au_project::aup3::save(Path::new(&a[3]), &p).unwrap();
        }
        "export" => {
            // export in.aup3 out.ext Key=value...  (keys = Audacity /FileFormats/ names)
            use au_io::export::{intfmt::*, mp3::*, ogg_opus::*};
            let p = au_project::aup3::load(Path::new(&a[2])).unwrap();
            let buf = p.state.tracks[0].clips.render();
            let tags = p.extras.tags.clone();
            let kv: std::collections::HashMap<String, String> = a[4..].iter().filter_map(|s| s.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let gi = |k: &str, d: i64| kv.get(k).map(|v| v.parse::<i64>().unwrap()).unwrap_or(d);
            let out = Path::new(&a[3]);
            let ext = out.extension().unwrap().to_str().unwrap().to_ascii_lowercase();
            let (ch, sr) = (&buf.channels[..], buf.sample_rate);
            let r = match ext.as_str() {
                "mp3" => {
                    let mut o = MP3Options::default();
                    o.mode = match kv.get("MP3RateModeChoice").map(String::as_str).unwrap_or("SET") { "VBR" => MP3RateMode::Variable, "ABR" => MP3RateMode::Average, "CBR" => MP3RateMode::Constant, _ => MP3RateMode::Preset };
                    o.quality_set = gi("MP3SetRate", 2) as i32; o.quality_vbr = gi("MP3VbrRate", 2) as i32;
                    o.bitrate_abr = gi("MP3AbrRate", 192) as i32; o.bitrate_cbr = gi("MP3CbrRate", 192) as i32;
                    export_mp3(out, ch, sr, &o, &tags)
                }
                "ogg" => export_vorbis(out, ch, sr, &VorbisOptions { quality: gi("OggExportQuality", 5) as i32 }, &tags),
                "opus" => {
                    let mut o = OpusOptions::default();
                    o.bitrate = match gi("OPUS/Bitrate", -1000) { 6000 => OpusBitrate::Kbps6, 8000 => OpusBitrate::Kbps8, 16000 => OpusBitrate::Kbps16, 24000 => OpusBitrate::Kbps24, 32000 => OpusBitrate::Kbps32, 40000 => OpusBitrate::Kbps40, 48000 => OpusBitrate::Kbps48, 64000 => OpusBitrate::Kbps64, 80000 => OpusBitrate::Kbps80, 96000 => OpusBitrate::Kbps96, 128000 => OpusBitrate::Kbps128, 160000 => OpusBitrate::Kbps160, 192000 => OpusBitrate::Kbps192, 256000 => OpusBitrate::Kbps256, -1 => OpusBitrate::Maximum, _ => OpusBitrate::Auto };
                    o.quality = gi("OPUS/Quality", 10) as i32;
                    o.vbr_mode = match gi("OPUS/VbrMode", 1) { 0 => OpusVbrMode::Off, 2 => OpusVbrMode::Constrained, _ => OpusVbrMode::On };
                    o.frame_duration = match gi("OPUS/FrameDuration", 200) { 25 => OpusFrameDuration::Ms2_5, 50 => OpusFrameDuration::Ms5, 100 => OpusFrameDuration::Ms10, 400 => OpusFrameDuration::Ms40, 600 => OpusFrameDuration::Ms60, _ => OpusFrameDuration::Ms20 };
                    o.application = match gi("OPUS/Application", 2049) { 2048 => OpusApplication::Speech, 2051 => OpusApplication::LowDelay, _ => OpusApplication::Audio };
                    o.cutoff = match gi("OPUS/Cutoff", -1000) { 1101 => OpusCutoff::Narrowband, 1102 => OpusCutoff::Mediumband, 1103 => OpusCutoff::Wideband, 1104 => OpusCutoff::SuperWideband, 1105 => OpusCutoff::Fullband, _ => OpusCutoff::Auto };
                    export_opus(out, ch, sr, &o, &tags)
                }
                "flac" => export_flac(out, ch, sr, &FlacOptions { bit_depth: gi("FLACBitDepth", 16) as u32, level: gi("FLACLevel", 5) as u32, ..Default::default() }, &tags),
                "wv" => {
                    let mut o = WavPackOptions::default();
                    o.quality = gi("WavPackEncodeQuality", o.quality as i64) as u32; o.bit_depth = gi("WavPackBitDepth", o.bit_depth as i64) as u32;
                    o.hybrid_mode = gi("WavPackHybridMode", 0) != 0; o.create_correction = gi("WavPackCreateCorrectionFile", 0) != 0;
                    o.bit_rate = gi("WavPackBitrate", o.bit_rate as i64) as u32;
                    export_wavpack(out, ch, sr, &o, &tags)
                }
                _ => {
                    let mut o = PcmOptions::default();
                    o.header = gi("ExportFormat_SF1", if ext == "aiff" { SF_FORMAT_AIFF as i64 } else { SF_FORMAT_WAV as i64 }) as i32;
                    o.encoding = gi("Encoding", SF_FORMAT_PCM_16 as i64) as i32;
                    export_pcm(out, ch, sr, &o, &tags)
                }
            };
            r.unwrap();
        }
        "aup3wav" => {
            let p = au_project::aup3::load(Path::new(&a[2])).unwrap();
            let ti: usize = a.get(4).map(|s| s.parse().unwrap()).unwrap_or(0);
            let buf = p.state.tracks[ti].clips.render();
            au_io::export_wav(Path::new(&a[3]), &buf, 32).unwrap();
            for lt in &p.state.labels { for l in &lt.labels { println!("LABEL\t{:.6}\t{:.6}\t{}", l.start, l.end, l.text); } }
        }
        "list" => {
            for e in registry::all() {
                let ps: Vec<String> = e.params().iter().map(|p| format!("{}={}", p.key, p.default)).collect();
                println!("{:?}\t{}\t{}\t{}", e.kind(), e.id(), e.supports_realtime(), ps.join(" "));
            }
        }
        _ => eprintln!("unknown command"),
    }
}
