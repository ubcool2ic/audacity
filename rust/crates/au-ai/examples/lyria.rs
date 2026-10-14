//! cargo run -p au-ai --example lyria -- <out-file> <instrumental:true|false> <prompt> [lyrics]
use au_ai::*;
fn main() -> Result<(), AiError> {
    let a: Vec<String> = std::env::args().collect();
    let gen = OpenRouterLyria::from_config(false)?;
    let req = MusicRequest { prompt: a[3].clone(), lyrics: a.get(4).cloned(), instrumental: a[2] == "true", duration_secs: 30.0, seed: None };
    println!("{}: est ${:.2}", gen.name(), gen.estimate_cost_usd(&req));
    let out = gen.generate(&req)?;
    std::fs::write(&a[1], &out.bytes)?;
    println!("wrote {} ({} bytes, {}), cost ${:.3}", a[1], out.bytes.len(), out.mime, out.est_cost_usd);
    Ok(())
}
