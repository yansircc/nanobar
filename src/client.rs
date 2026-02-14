use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::daemon;

pub fn send_command(cmd: &str) -> Result<String> {
    let path = daemon::socket_path();
    let stream =
        UnixStream::connect(&path).context("daemon not running (use 'nanobar start' first)")?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();

    let mut writer = &stream;
    writer
        .write_all(format!("{}\n", cmd).as_bytes())
        .context("failed to send command")?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .context("failed to read response")?;

    Ok(response.trim().to_string())
}

pub fn is_daemon_running() -> bool {
    send_command("ping").map(|r| r == "pong").unwrap_or(false)
}
