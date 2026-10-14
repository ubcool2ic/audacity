//! ny_run <script.ny> <in.wav|-> <out.wav> <t0> <t1> [key=value ...]
//! Runs a .ny through the Audacity-exact engine (in-process). Float32 WAV I/O.
//! Prints LABEL\tt0\tt1\ttext lines and TEXT output to stdout.
use au_core::AudioBuffer;
use au_plugin_host::nyquist::{NyquistEngine, NyquistJob, NyquistOutput};

fn read_wav(p: &str) -> AudioBuffer {
    let b = std::fs::read(p).expect("read wav");
    let (mut i, mut ch, mut sr) = (12usize, 1usize, 44100u32);
    let u32at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    while i + 8 <= b.len() {
        let sz = u32at(i + 4) as usize;
        if &b[i..i + 4] == b"fmt " {
            ch = u16::from_le_bytes([b[i + 10], b[i + 11]]) as usize;
            sr = u32at(i + 12);
        } else if &b[i..i + 4] == b"data" {
            let mut chans = vec![Vec::new(); ch];
            for (k, s) in b[i + 8..i + 8 + sz].chunks_exact(4).enumerate() {
                chans[k % ch].push(f32::from_le_bytes(s.try_into().unwrap()));
            }
            return AudioBuffer::from_channels(sr, chans);
        }
        i += 8 + sz + (sz & 1);
    }
    panic!("no data chunk")
}

fn write_wav(p: &str, a: &AudioBuffer) {
    let (ch, n) = (a.channels.len(), a.len());
    let mut d = Vec::with_capacity(n * ch * 4);
    for f in 0..n {
        for c in &a.channels {
            d.extend_from_slice(&c[f].to_le_bytes());
        }
    }
    let mut o = Vec::new();
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&(36 + d.len() as u32).to_le_bytes());
    o.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] { o.extend_from_slice(&v.to_le_bytes()); }
    o.extend_from_slice(&3u16.to_le_bytes());
    o.extend_from_slice(&(ch as u16).to_le_bytes());
    o.extend_from_slice(&a.sample_rate.to_le_bytes());
    o.extend_from_slice(&(a.sample_rate * ch as u32 * 4).to_le_bytes());
    o.extend_from_slice(&((ch * 4) as u16).to_le_bytes());
    o.extend_from_slice(&32u16.to_le_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&(d.len() as u32).to_le_bytes());
    o.extend_from_slice(&d);
    std::fs::write(p, o).expect("write wav");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let audio = (a[2] != "-").then(|| read_wav(&a[2]));
    let params = a[6..]
        .iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect();
    let job = NyquistJob {
        script_path: a[1].clone().into(),
        params,
        audio,
        selection_secs: (a[4].parse().unwrap(), a[5].parse().unwrap()),
    };
    match au_nyquist::AudacityNyquist::default().run(&job, &|| false) {
        Ok(NyquistOutput::Audio(b)) => write_wav(&a[3], &b),
        Ok(NyquistOutput::Labels(l)) => l.iter().for_each(|(s, e, t)| println!("LABEL\t{s}\t{e}\t{t}")),
        Ok(NyquistOutput::Text(t)) => println!("TEXT\t{t}"),
        Ok(NyquistOutput::Nothing) => println!("NOTHING"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}
