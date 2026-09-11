use anyhow::Result;
use std::{
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);
#[derive(Debug)]
pub struct Interrupted;
impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Interrupted.")
    }
}
impl std::error::Error for Interrupted {}
pub fn check_interrupt() -> Result<()> {
    if INTERRUPTED.load(Ordering::Relaxed) {
        return Err(Interrupted.into());
    }
    Ok(())
}
#[derive(Debug)]
pub struct TimedOut;
impl fmt::Display for TimedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Command timed out.")
    }
}
impl std::error::Error for TimedOut {}
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}
fn read(mut file: File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// File-backed streams avoid pipe deadlocks and keep child cleanup synchronous.
pub fn capture(
    command: &mut Command,
    input: Option<&str>,
    inherit_stderr: bool,
    timeout: Option<Duration>,
) -> Result<Output> {
    check_interrupt()?;
    let stdout = tempfile::tempfile()?;
    let stderr = tempfile::tempfile()?;
    command.stdout(Stdio::from(stdout.try_clone()?));
    if inherit_stderr {
        command.stderr(Stdio::inherit());
    } else {
        command.stderr(Stdio::from(stderr.try_clone()?));
    }
    if let Some(input) = input {
        let mut file = tempfile::tempfile()?;
        file.write_all(input.as_bytes())?;
        file.seek(SeekFrom::Start(0))?;
        command.stdin(Stdio::from(file));
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Err(error) = check_interrupt() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if timeout.is_some_and(|t| started.elapsed() >= t) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: read(stdout)?,
        stderr: read(stderr)?,
    })
}
