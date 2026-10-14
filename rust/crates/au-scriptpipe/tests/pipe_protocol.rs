#![cfg(unix)]

use au_scriptpipe::{
    finished, serve_with, PipeClient, PipeNames, PipeServer, Request, RESPONSE_FAILED,
    RESPONSE_OK,
};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "au-scriptpipe-test-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn names(&self) -> PipeNames {
        PipeNames::in_dir(&self.0, "protocol")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn finished_formats_output_and_status() {
    assert_eq!(finished("", true), RESPONSE_OK);
    assert_eq!(finished("output", true), format!("output\n{RESPONSE_OK}"));
    assert_eq!(
        finished("output\n", false),
        format!("output\n{RESPONSE_FAILED}")
    );
}

#[test]
fn round_trip_of_three_commands() {
    let dir = TestDir::new();
    let server = serve_with(dir.names(), |command| {
        finished(&format!("Echo: {command}"), true)
    })
    .expect("start server");

    let mut client =
        PipeClient::connect(server.names(), Duration::from_secs(2)).expect("connect client");

    for command in ["First", "Second", "Third"] {
        let response = client.command(command).expect("command response");
        assert!(response.contains(&format!("Echo: {command}")));
        assert!(response.ends_with(RESPONSE_OK));
    }

    drop(client);
    server.stop();
}

#[test]
fn serves_a_second_client_after_disconnect() {
    let dir = TestDir::new();
    let server = serve_with(dir.names(), |command| finished(command, true))
        .expect("start server");

    let mut first =
        PipeClient::connect(server.names(), Duration::from_secs(2)).expect("connect first client");
    assert!(first
        .command("first client")
        .expect("first response")
        .contains("first client"));
    drop(first);

    let mut second =
        PipeClient::connect(server.names(), Duration::from_secs(2)).expect("connect second client");
    let response = second.command("second client").expect("second response");
    assert!(response.contains("second client"));
    assert!(response.ends_with(RESPONSE_OK));

    drop(second);
    server.stop();
}

#[test]
fn dropping_request_sends_failure_response() {
    let dir = TestDir::new();
    let (server, requests) =
        PipeServer::start(dir.names(), None).expect("start server");

    let names = server.names().clone();
    let client = thread::spawn(move || {
        let mut client = PipeClient::connect(&names, Duration::from_secs(2)).expect("connect client");
        client.command("unanswered").expect("failure response")
    });
    let request: Request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("receive request");
    assert_eq!(request.command(), "unanswered");
    drop(request);
    let response = client.join().expect("client thread");
    assert!(response.contains("unanswered was not answered"));
    assert!(response.ends_with(RESPONSE_FAILED));

    server.stop();
}

#[test]
fn crlf_and_nul_terminated_commands_are_normalized() {
    let dir = TestDir::new();
    let server = serve_with(dir.names(), |command| finished(command, true))
        .expect("start server");

    let mut client =
        PipeClient::connect(server.names(), Duration::from_secs(2)).expect("connect client");
    let response = client
        .command("normalized\r\0")
        .expect("normalized response");

    assert!(response.starts_with("normalized\n"));
    assert!(response.ends_with(RESPONSE_OK));

    drop(client);
    server.stop();
}

#[test]
fn stop_without_a_client_removes_fifos_promptly() {
    let dir = TestDir::new();
    let names = dir.names();
    let server = serve_with(names.clone(), |command| finished(command, true))
        .expect("start server");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !names.to_server.exists() || !names.from_server.exists() {
        assert!(Instant::now() < deadline, "server did not create FIFOs");
        thread::sleep(Duration::from_millis(5));
    }

    let started = Instant::now();
    server.stop();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!names.to_server.exists());
    assert!(!names.from_server.exists());
}

#[test]
fn audacity_pipe_test_py_compatible() {
    let Some(script) = std::env::var_os("AU_PIPE_TEST_PY") else {
        println!("skipping Audacity pipe_test.py compatibility test: AU_PIPE_TEST_PY is unset");
        return;
    };

    if Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        println!("skipping Audacity pipe_test.py compatibility test: python3 is unavailable");
        return;
    }

    let help = "{ \"name\":\"Help\", \"parameters\": [] }".to_owned();
    let server = serve_with(PipeNames::audacity_default(), move |_| {
        finished(&help, true)
    })
    .expect("start default-name server");

    let mut child = Command::new("python3")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start pipe_test.py");

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().expect("poll pipe_test.py").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            server.stop();
            panic!("pipe_test.py did not finish within 20 seconds");
        }
        thread::sleep(Duration::from_millis(25));
    }

    let output = child.wait_with_output().expect("collect pipe_test.py output");
    server.stop();

    assert!(
        output.status.success(),
        "pipe_test.py failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.matches(RESPONSE_OK).count() >= 2,
        "expected two successful responses in stdout, got: {stdout}"
    );
}

#[test]
fn request_channel_is_usable_without_a_handler_helper() {
    let dir = TestDir::new();
    let (server, requests) = PipeServer::start(dir.names(), None).expect("start server");
    let mut client =
        PipeClient::connect(server.names(), Duration::from_secs(2)).expect("connect client");

    let (sent, received) = mpsc::channel();
    let responder = thread::spawn(move || {
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("receive request");
        sent.send(request.command().to_owned()).expect("send command");
        request.respond(&finished("manual response", true));
    });

    let response = client.command("manual").expect("manual response");
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).expect("command"),
        "manual"
    );
    assert!(response.contains("manual response"));
    assert!(response.ends_with(RESPONSE_OK));

    responder.join().expect("join responder");
    drop(client);
    server.stop();
}


