//! Minimal Audacity-compatible pipe echo server.

use au_scriptpipe::{finished, serve_with, PipeNames};
use std::thread;
use std::time::Duration;

fn main() {
    let server = serve_with(PipeNames::audacity_default(), |command| {
        finished(&format!("Echo: {command}"), true)
    })
    .expect("start mod-script-pipe echo server");

    println!("mod-script-pipe echo server is running; press Ctrl-C to stop");

    // The host application normally owns the server's lifetime. This example
    // keeps it alive until the process receives Ctrl-C.
    loop {
        thread::sleep(Duration::from_secs(1));
        let _ = server.names();
    }
}
