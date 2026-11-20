use std::{process::Command, time::Instant};

use anyhow::{Context, Result, bail};

use crate::config::NAMESPACE;

// zenity exits with 1 when the dialog is cancelled
const ZENITY_CANCELLED: i32 = 1;

pub fn stdout_of(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

// None when the dialog is cancelled
pub fn zenity(program: &str, args: &[&str]) -> Result<Option<String>> {
    let out = Command::new(program)
        .args(args)
        .output()
        .context("could not run zenity")?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())),
        Some(ZENITY_CANCELLED) => Ok(None),
        _ => bail!(
            "zenity failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

pub fn notify(desktop: bool, summary: &str, body: &str, icon: Option<&str>) {
    eprintln!("{summary}: {body}");
    if !desktop {
        return;
    }
    let mut command = Command::new("notify-send");
    command.args(["-a", NAMESPACE]);
    if let Some(icon) = icon {
        command.args(["-i", icon]);
    }
    if let Err(e) = command.args([summary, body]).status() {
        eprintln!("notify-send: {e}");
    }
}

// --profile-startup: prints the time since launch at each stage, nothing when disabled
pub struct Stopwatch {
    start: Option<Instant>,
}

impl Stopwatch {
    pub fn new(enabled: bool) -> Self {
        Self {
            start: enabled.then(Instant::now),
        }
    }

    pub fn mark(&self, stage: &str) {
        if let Some(start) = self.start {
            eprintln!("{stage}: {:?}", start.elapsed());
        }
    }

    // the last stage, after which the stopwatch stays quiet
    pub fn first_frame(&mut self) {
        self.mark("first frame");
        self.start = None;
    }
}
