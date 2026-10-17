use anyhow::{Context, Result, bail};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

fn read_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > limit {
            return Err(std::io::Error::other(format!(
                "command output exceeds {} MiB",
                limit / 1024 / 1024
            )));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

pub fn run_cancellable(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
    cancelled: &AtomicBool,
    label: &str,
) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .with_context(|| format!("start {label}"))?;
    let stdout = child.stdout.take().context("missing command stdout")?;
    let stderr = child.stderr.take().context("missing command stderr")?;
    let out = thread::spawn(move || read_bounded(stdout, limit));
    let err = thread::spawn(move || read_bounded(stderr, limit));
    let started = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Err(anyhow::anyhow!("{label} cancelled"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            Ok(None) => break Err(anyhow::anyhow!("{label} timed out")),
            Err(error) => break Err(error.into()),
        }
    };
    // Reap descendants too, including those holding inherited output pipes open.
    // SAFETY: the child owns this newly created process group; no pointers are passed.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let _ = child.wait();
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;
    let status = status?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    let label = command.get_program().to_string_lossy().into_owned();
    let Output {
        status,
        stdout,
        stderr,
    } = run_cancellable(
        command,
        timeout,
        OUTPUT_LIMIT,
        &AtomicBool::new(false),
        &label,
    )?;
    if !status.success() {
        bail!(
            "{label} failed ({status}): {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(stdout)
}
