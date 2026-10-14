use au_runtime::compute::{self, Exec, MixInput};
use au_runtime::{CancelToken, JobError, Progress, Runtime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n).map(|_| { x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((x >> 40) as f32 / (1u64 << 23) as f32) - 1.0 }).collect()
}
fn bits(v: &[f32]) -> Vec<u32> { v.iter().map(|f| f.to_bits()).collect() }

#[test]
fn mix_parallel_is_bit_identical() {
    let rt = Runtime::new(8, 4);
    let tracks: Vec<Vec<Vec<f32>>> = (0..12).map(|t| (0..1 + t % 2).map(|c| noise(t * 7 + c, 300_000 + t as usize * 1777)).collect()).collect();
    let inputs: Vec<MixInput> = tracks.iter().enumerate().map(|(i, ch)| MixInput {
        channels: ch.iter().map(|c| c.as_slice()).collect(),
        gain: 0.1 + i as f32 * 0.07, pan: (i as f32 / 6.0) - 1.0, offset: i * 3001,
    }).collect();
    let frames = 360_000;
    let (c, p) = (CancelToken::new(), Progress::new(frames as u64));
    let s = compute::mix_offline(Exec::Serial, &inputs, frames, &c, &p).unwrap();
    let par = rt.pool().install(|| compute::mix_offline(Exec::Parallel, &inputs, frames, &c, &Progress::new(0)).unwrap());
    assert_eq!(bits(&s[0]), bits(&par[0]));
    assert_eq!(bits(&s[1]), bits(&par[1]));
    assert_eq!(p.done(), frames as u64);
    assert!(s[0].iter().any(|&x| x != 0.0));
}

#[test]
fn summaries_spectrogram_and_map_are_bit_identical() {
    let rt = Runtime::new(6, 4);
    let x = noise(42, 1_000_003);
    for block in [64usize, 256, 65_536] {
        let a = compute::waveform_summary(Exec::Serial, &x, block);
        let b = rt.pool().install(|| compute::waveform_summary(Exec::Parallel, &x, block));
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(&b) {
            assert_eq!((a.min.to_bits(), a.max.to_bits(), a.rms.to_bits()), (b.min.to_bits(), b.max.to_bits(), b.rms.to_bits()));
        }
    }
    let c = CancelToken::new();
    let a = compute::spectrogram(Exec::Serial, &x[..200_000], 1024, 256, &c).unwrap();
    let b = rt.pool().install(|| compute::spectrogram(Exec::Parallel, &x[..200_000], 1024, 256, &c).unwrap());
    assert_eq!(a.len(), b.len());
    assert!(a.iter().zip(&b).all(|(a, b)| bits(a) == bits(b)));
    // map_items stands in for per-track effect render / per-file decode.
    let items: Vec<Vec<f32>> = (0..20).map(|i| noise(i, 50_000)).collect();
    let render = |v: &Vec<f32>| Ok::<_, JobError>(v.iter().map(|s| (s * 1.7).tanh()).collect::<Vec<f32>>());
    let a = compute::map_items(Exec::Serial, &items, &c, &Progress::default(), render).unwrap();
    let b = rt.pool().install(|| compute::map_items(Exec::Parallel, &items, &c, &Progress::default(), render).unwrap());
    assert!(a.iter().zip(&b).all(|(a, b)| bits(a) == bits(b)));
}

#[test]
fn stress_many_jobs_bounded_queue_cancel_and_panics() {
    let rt = Runtime::new(4, 8);
    let ran = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..2_000u64 {
        let ran = ran.clone();
        handles.push(rt.submit(100, move |ctx| {
            if i % 97 == 13 { panic!("injected worker panic {i}"); }
            let data = noise(i, 4_096);
            let r = ctx.install(|| compute::waveform_summary(Exec::Parallel, &data, 64));
            ctx.progress.add(100);
            ran.fetch_add(1, Ordering::Relaxed);
            Ok(r.len())
        }));
    }
    let (mut ok, mut panicked) = (0, 0);
    for h in handles {
        match h.wait() { Ok(n) => { assert_eq!(n, 64); ok += 1 } Err(JobError::Panicked(m)) => { assert!(m.contains("injected")); panicked += 1 } Err(e) => panic!("{e}") }
    }
    assert_eq!(panicked, (0..2_000u64).filter(|i| i % 97 == 13).count());
    assert_eq!(ok + panicked, 2_000);
    assert_eq!(ran.load(Ordering::Relaxed), ok);

    // Cancellation: a long job stops at its next check.
    let h = rt.submit(0, |ctx| { loop { ctx.cancel.check()?; std::thread::yield_now(); } #[allow(unreachable_code)] Ok(()) });
    std::thread::sleep(std::time::Duration::from_millis(20));
    h.cancel.cancel();
    assert_eq!(h.wait(), Err(JobError::Cancelled));

    // Backpressure: with all workers blocked and queue full, try_submit says Busy.
    let rt = Runtime::new(1, 2);
    let gate = Arc::new(std::sync::Barrier::new(2));
    let g = gate.clone();
    let blocker = rt.submit(0, move |_| { g.wait(); Ok(()) });
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut queued = Vec::new();
    let mut busy = false;
    for _ in 0..10 { match rt.try_submit(0, |_| Ok(())) { Ok(h) => queued.push(h), Err(JobError::Busy) => { busy = true; break } Err(e) => panic!("{e}") } }
    assert!(busy && queued.len() <= 3, "queue not bounded: {}", queued.len());
    gate.wait();
    blocker.wait().unwrap();
    for h in queued { h.wait().unwrap(); }
}
