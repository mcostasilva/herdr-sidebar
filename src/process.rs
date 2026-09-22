//! Bounded subprocess execution. Interactive OpenCode uses exec instead.

use anyhow::{Context, Result, bail, ensure};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde::de::DeserializeOwned;
use std::{
    io::{self, Read},
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
pub const POLL: Duration = Duration::from_millis(20);

struct Running(Child, bool);

impl Drop for Running {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        // The command gets a fresh process group. Kill its descendants too:
        // otherwise they can keep stdout/stderr pipes open after a timeout.
        let _ = killpg(Pid::from_raw(self.0.id() as i32), Signal::SIGKILL);
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_pipe(pipe: impl Read + Send + 'static) -> Receiver<io::Result<Vec<u8>>> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    receive
}

pub fn json<T: DeserializeOwned>(command: &mut Command, timeout: Duration) -> Result<T> {
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = Running(
        command
            .spawn()
            .with_context(|| format!("start {program}"))?,
        false,
    );
    let stdout = read_pipe(child.0.stdout.take().context("missing stdout pipe")?);
    let stderr = read_pipe(child.0.stderr.take().context("missing stderr pipe")?);
    let started = Instant::now();
    let (mut out, mut err) = (None, None);
    let mut status = None;
    loop {
        if out.is_none() {
            out = stdout.try_recv().ok();
        }
        if err.is_none() {
            err = stderr.try_recv().ok();
        }
        if status.is_none() {
            status = child.0.try_wait()?;
        }
        if out.is_some() && err.is_some() && status.is_some() {
            break;
        }
        ensure!(
            started.elapsed() < timeout,
            "{program} timed out after {} ms",
            timeout.as_millis()
        );
        thread::sleep(POLL);
    }
    child.1 = true;
    let out = out.context("missing stdout")??;
    let err = err.context("missing stderr")??;
    ensure!(
        out.len() <= OUTPUT_LIMIT && err.len() <= OUTPUT_LIMIT,
        "{program} output exceeded 4 MiB"
    );
    let status = status.context("missing command status")?;
    ensure!(
        status.success(),
        "{program} failed ({status}): {}{}",
        String::from_utf8_lossy(&err),
        String::from_utf8_lossy(&out)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&out).with_context(|| format!("invalid JSON from {program}"))?;
    if let Some(error) = value.get("error") {
        bail!("{program}: {error}");
    }
    serde_json::from_value(value).with_context(|| format!("unexpected response from {program}"))
}
