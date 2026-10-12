use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn run_until_ready(exe: &str, session: &Path, bind: &str) -> String {
    let mut child = Command::new(exe)
        .args([
            "run",
            "--session",
            session.to_str().expect("session utf8"),
            "--bind",
            bind,
            "--renderer",
            "dx12",
        ])
        .env("EIVIZ_PLAN", "professional")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn eiviz-headless");
    let stderr = child.stderr.take().expect("stderr");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = std::sync::Arc::clone(&collected);
    let reader = thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let mut file = stderr;
        let mut bytes = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match file.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes[..n]));
                    if buf.contains("eiviz-headless ready") {
                        break;
                    }
                    if buf.contains("eiviz-headless error=") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        *sink.lock().expect("stderr") = buf;
    });
    let _ = reader.join();
    let _ = child.kill();
    let _ = child.wait();
    collected.lock().expect("log").clone()
}

#[test]
fn bars_headless_run_reaches_ready_without_session_error() {
    let exe = env!("CARGO_BIN_EXE_eiviz-headless");
    let session = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bars.eivz");
    let log = run_until_ready(exe, &session, "127.0.0.1:19411");
    assert!(
        log.contains("eiviz-headless ready"),
        "headless did not become ready:\n{log}"
    );
    assert!(
        !log.contains("eiviz-headless error=session"),
        "session apply failed:\n{log}"
    );
}

#[test]
fn incomplete_pro_outputs_do_not_abort_headless() {
    let exe = env!("CARGO_BIN_EXE_eiviz-headless");
    let dir = std::env::temp_dir().join(format!("eiviz-headless-pro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let session = dir.join("pro-io.eivz");
    let doc = eiviz_control::session::parse(
        br#"{
          "version": 2,
          "settings": { "masterFpsNum": 60000, "masterFpsDen": 1001, "vmixApiEnabled": false, "vmixTcpEnabled": false, "nativeApiEnabled": false },
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }],
          "outputs": [
            { "id": 100, "name": "pgm", "transport": "Omt", "sourceKind": "MuProgram" },
            { "id": 101, "name": "deck", "transport": "DeckLink", "sourceKind": "MuProgram" },
            { "id": 102, "name": "yt", "transport": "Rtmp", "streamEncoderId": 1 },
            { "id": 103, "name": "dead", "transport": "Rtmp", "streamEncoderId": 1, "rtmpUrl": "rtmp://127.0.0.1:1/live/x" }
          ]
        }"#,
    )
    .expect("parse");
    eiviz_control::session::write_document(&session, &doc).expect("write");
    let log = run_until_ready(exe, &session, "127.0.0.1:19412");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        log.contains("eiviz-headless ready"),
        "headless did not become ready:\n{log}"
    );
    assert!(
        !log.contains("eiviz-headless error=session"),
        "incomplete DeckLink/RTMP aborted headless:\n{log}"
    );
}
