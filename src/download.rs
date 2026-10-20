//! Worker threads. Each download runs yt-dlp in its own thread and reports back over a
//! channel; the UI thread only ever calls `try_recv`, so it never waits on yt-dlp.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::ytdlp::{self, Parsed, Progress};

/// Identifies one run of one job. `run` increases on every retry so that late messages
/// from an older run can be told apart from the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    pub id: u64,
    pub run: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Exit {
    Success,
    Failed(Option<i32>),
    Cancelled,
    SpawnFailed(String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    Progress(Tag, Progress),
    Final(Tag, String),
    Phase(Tag, &'static str),
    Log { tag: Tag, line: String, is_stderr: bool },
    Done(Tag, Exit),
}

impl Msg {
    pub fn tag(&self) -> Tag {
        match self {
            Msg::Progress(t, _) | Msg::Final(t, _) | Msg::Phase(t, _) | Msg::Done(t, _) => *t,
            Msg::Log { tag, .. } => *tag,
        }
    }
}

/// Start a download on a new thread. Setting `cancel` makes the worker kill yt-dlp.
pub fn spawn_download(tag: Tag, program: String, args: Vec<String>, tx: Sender<Msg>, cancel: Arc<AtomicBool>) {
    let tx_err = tx.clone();
    let spawned =
        thread::Builder::new().name(format!("download-{}", tag.id)).spawn(move || run(tag, program, args, tx, cancel));
    if let Err(e) = spawned {
        let _ = tx_err.send(Msg::Done(tag, Exit::SpawnFailed(format!("could not start worker thread: {e}"))));
    }
}

fn run(tag: Tag, program: String, args: Vec<String>, tx: Sender<Msg>, cancel: Arc<AtomicBool>) {
    let mut cmd = Command::new(&program);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        // Own process group, so cancelling can stop yt-dlp and the ffmpeg it started.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                format!("could not start '{program}': not found. Install yt-dlp or set its path in Settings.")
            } else {
                format!("could not start '{program}': {e}")
            };
            let _ = tx.send(Msg::Done(tag, Exit::SpawnFailed(msg)));
            return;
        }
    };

    let readers: Vec<JoinHandle<()>> = [
        child.stdout.take().map(|s| spawn_reader(tag, s, false, tx.clone())),
        child.stderr.take().map(|s| spawn_reader(tag, s, true, tx.clone())),
    ]
    .into_iter()
    .flatten()
    .collect();

    let exit = loop {
        if cancel.load(Ordering::SeqCst) {
            kill_tree(&mut child);
            let _ = child.wait();
            break Exit::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Exit::Success,
            Ok(Some(status)) => break Exit::Failed(status.code()),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => break Exit::SpawnFailed(format!("waiting for yt-dlp failed: {e}")),
        }
    };

    // After a normal exit the pipes are closed, so the readers finish on their own and
    // every log line is sent before `Done`. After a cancel we do not wait for them.
    if exit != Exit::Cancelled {
        for r in readers {
            let _ = r.join();
        }
    }
    let _ = tx.send(Msg::Done(tag, exit));
}

fn spawn_reader<R: Read + Send + 'static>(tag: Tag, source: R, is_stderr: bool, tx: Sender<Msg>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(source);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            // ffmpeg separates its status updates with \r, so split on both.
            for piece in text.split(['\r', '\n']) {
                if piece.trim().is_empty() {
                    continue;
                }
                let sent = match ytdlp::parse_line(piece) {
                    Parsed::Progress(p) => tx.send(Msg::Progress(tag, p)),
                    Parsed::Final(path) => tx.send(Msg::Final(tag, path)),
                    Parsed::Text { text, phase } => {
                        if let Some(p) = phase {
                            let _ = tx.send(Msg::Phase(tag, p));
                        }
                        tx.send(Msg::Log { tag, line: text, is_stderr })
                    }
                };
                if sent.is_err() {
                    return; // the UI is gone
                }
            }
        }
    })
}

/// Stop the process and everything it started.
fn kill_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-s", "TERM", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::time::Instant;

    fn script(name: &str, body: &str) -> String {
        let path = std::env::temp_dir().join(format!("ytget-rs-test-{}-{name}", std::process::id()));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn collect_until_done(rx: &mpsc::Receiver<Msg>, limit: Duration) -> Vec<Msg> {
        let start = Instant::now();
        let mut out = Vec::new();
        while start.elapsed() < limit {
            if let Ok(m) = rx.recv_timeout(Duration::from_millis(100)) {
                let done = matches!(m, Msg::Done(..));
                out.push(m);
                if done {
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn reports_progress_final_path_and_exit() {
        let prog =
            script("ok", "echo 'PROG|1000|NA|2000|500.5|3|A | B'\necho 'FINAL|/tmp/x.mkv'\necho 'ERROR: boom' >&2");
        let (tx, rx) = mpsc::channel();
        let tag = Tag { id: 1, run: 1 };
        spawn_download(tag, prog, vec![], tx, Arc::new(AtomicBool::new(false)));
        let msgs = collect_until_done(&rx, Duration::from_secs(5));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Progress(_, p) if p.total == Some(2000.0))));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Final(_, p) if p == "/tmp/x.mkv")));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Log { line, is_stderr: true, .. } if line == "ERROR: boom")));
        assert!(matches!(msgs.last(), Some(Msg::Done(_, Exit::Success))));
    }

    #[test]
    fn nonzero_exit_is_a_failure() {
        let prog = script("fail", "exit 3");
        let (tx, rx) = mpsc::channel();
        spawn_download(Tag { id: 2, run: 1 }, prog, vec![], tx, Arc::new(AtomicBool::new(false)));
        let msgs = collect_until_done(&rx, Duration::from_secs(5));
        assert!(matches!(msgs.last(), Some(Msg::Done(_, Exit::Failed(Some(3))))));
    }

    #[test]
    fn cancel_kills_the_child_and_its_children() {
        let prog = script("slow", "sleep 30 &\nwait");
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        spawn_download(Tag { id: 3, run: 1 }, prog, vec![], tx, cancel.clone());
        thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        cancel.store(true, Ordering::SeqCst);
        let msgs = collect_until_done(&rx, Duration::from_secs(5));
        assert!(matches!(msgs.last(), Some(Msg::Done(_, Exit::Cancelled))));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn missing_program_is_reported_clearly() {
        let (tx, rx) = mpsc::channel();
        spawn_download(
            Tag { id: 4, run: 1 },
            "/definitely/not/here/yt-dlp".into(),
            vec![],
            tx,
            Arc::new(AtomicBool::new(false)),
        );
        let msgs = collect_until_done(&rx, Duration::from_secs(5));
        match msgs.last() {
            Some(Msg::Done(_, Exit::SpawnFailed(m))) => assert!(m.contains("not found")),
            other => panic!("unexpected {other:?}"),
        }
    }
}
