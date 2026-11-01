#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::json;

fn drain(
    receiver: &mpsc::Receiver<Vec<u8>>,
    parser: &mut vt100::Parser,
    raw: &mut Vec<u8>,
    window: Duration,
) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(chunk) => {
                raw.extend_from_slice(&chunk);
                parser.process(&chunk);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn artifact_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("pty-artifacts")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_dashboard_renders_navigates_and_exits() {
    let server = support::start_interactive_stub_daemon().await;
    let rows = NonZeroU16::new(40).expect("rows");
    let cols = NonZeroU16::new(120).expect("cols");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.get(),
            cols: cols.get(),
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_knect"));
    command.args(["--daemon-url", server.url.as_str()]);
    command.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(command).expect("spawn knect");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    drop(pair.slave);

    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if sender.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut parser = vt100::Parser::new(rows, cols, 0);
    let mut raw = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        drain(&receiver, &mut parser, &mut raw, Duration::from_millis(250));
        let screen = parser.screen().contents();
        if screen.contains("Topology") && screen.contains("hello-00001") {
            break;
        }
        if Instant::now() > deadline {
            panic!("dashboard did not render before the deadline:\n{screen}");
        }
    }

    writer.write_all(b"j").expect("send j");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(400));
    writer.write_all(b"k").expect("send k");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(400));
    writer.write_all(b"2").expect("send 2");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(500));

    let dashboard = parser.screen().contents();
    for expected in ["Resources", "Topology", "Inspector", "Logs"] {
        assert!(
            dashboard.contains(expected),
            "missing pane {expected} in PTY capture:\n{dashboard}"
        );
    }
    let directory = artifact_directory();
    std::fs::create_dir_all(&directory).expect("artifact directory");
    std::fs::write(directory.join("knect-pty-dashboard.txt"), &dashboard)
        .expect("write text artifact");
    std::fs::write(directory.join("knect-pty-dashboard.ansi"), &raw).expect("write ansi artifact");

    writer.write_all(b"d").expect("send d");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(900));
    let diff = parser.screen().contents();
    for expected in [
        "Mutation diff",
        "User intent",
        "Knative defaults",
        "Policy mutators",
        "knative-serving-controller",
    ] {
        assert!(
            diff.contains(expected),
            "missing {expected} in diff modal:\n{diff}"
        );
    }
    std::fs::write(directory.join("knect-pty-diff.txt"), &diff).expect("write diff artifact");
    writer.write_all(b"\x1b").expect("send Esc");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(300));

    writer.write_all(b"t").expect("send t");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(700));
    let traffic = parser.screen().contents();
    for expected in ["Traffic split", "hello-00001", "total 100%", "valid"] {
        assert!(
            traffic.contains(expected),
            "missing {expected} in traffic modal:\n{traffic}"
        );
    }
    std::fs::write(directory.join("knect-pty-traffic.txt"), &traffic)
        .expect("write traffic artifact");
    writer.write_all(b"\x1b").expect("send Esc");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(300));

    writer.write_all(b"\t\t").expect("focus the tree pane");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(250));
    writer.write_all(b"jjj").expect("select broker");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(400));
    writer.write_all(b"w").expect("send w");
    writer.flush().expect("flush");
    let tap_deadline = Instant::now() + Duration::from_secs(6);
    loop {
        drain(&receiver, &mut parser, &mut raw, Duration::from_millis(300));
        let screen = parser.screen().contents();
        if screen.contains("Tap — Broker default/orders")
            && screen.contains("dev.knative.samples.orders.created")
        {
            break;
        }
        if Instant::now() > tap_deadline {
            panic!("tap modal did not render events:\n{screen}");
        }
    }
    let tap = parser.screen().contents();
    assert!(tap.contains("ce-"), "tap modal did not show ce-id:\n{tap}");
    std::fs::write(directory.join("knect-pty-tap.txt"), &tap).expect("write tap artifact");
    writer.write_all(b"\x1b").expect("send Esc");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(300));

    writer.write_all(b"?").expect("send ?");
    writer.flush().expect("flush");
    drain(&receiver, &mut parser, &mut raw, Duration::from_millis(500));
    let help = parser.screen().contents();
    assert!(help.contains("knect help"), "help overlay missing:\n{help}");
    std::fs::write(directory.join("knect-pty-help.txt"), &help).expect("write help artifact");

    let metadata = json!({
        "daemon_url": server.url,
        "rows": rows.get(),
        "cols": cols.get(),
        "keys": ["j", "k", "2", "d", "t", "w", "?", "Esc", "q"],
        "artifacts": [
            "knect-pty-dashboard.txt",
            "knect-pty-dashboard.ansi",
            "knect-pty-diff.txt",
            "knect-pty-traffic.txt",
            "knect-pty-tap.txt",
            "knect-pty-help.txt"
        ],
    });
    std::fs::write(
        directory.join("metadata.json"),
        serde_json::to_string_pretty(&metadata).expect("metadata"),
    )
    .expect("write metadata");

    writer.write_all(b"\x1b").expect("send Esc");
    writer.write_all(b"q").expect("send q");
    writer.flush().expect("flush");

    let exit_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            assert!(status.success(), "knect exited with {status:?}");
            break;
        }
        if Instant::now() > exit_deadline {
            let _ = child.kill();
            panic!("knect did not exit after q");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
