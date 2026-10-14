//! cargo run -p au-ai --example instrumental -- <python> <in.wav> <out.wav>
use au_ai::*;
use std::path::{Path, PathBuf};
fn main() -> Result<(), AiError> {
    let a: Vec<String> = std::env::args().collect();
    let sep = DemucsSeparator { python: a[1].clone(), model: "htdemucs".into(), out_dir: PathBuf::from("/tmp/au-ai-stems") };
    let t = std::time::Instant::now();
    let s = export_instrumental(&sep, Path::new(&a[2]), Path::new(&a[3]))?;
    println!("separated {} frames @{}Hz in {:.1}s", s.drums.len() / s.channels as usize, s.sample_rate, t.elapsed().as_secs_f32());
    Ok(())
}
