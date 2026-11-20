use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tiny_skia::{IntRect, Pixmap};

use crate::{
    config::{
        DATE_FIELDS, DEFAULT_OUTPUT_FILENAME, PRIVATE_FILE_MODE, STDIO_PATH,
        THUMBNAIL_MAX_AGE_SECS, THUMBNAIL_PREFIX, WL_COPY,
    },
    util::stdout_of,
};

pub fn crop_png(image: &Pixmap, region: IntRect) -> Result<Vec<u8>> {
    let cropped = image
        .clone_rect(region)
        .context("selection is outside the image")?;
    cropped.encode_png().context("could not encode the PNG")
}

// wl-copy forks and keeps serving, so it must not hold our pipes
pub fn copy(data: &[u8], mime: &str, command: Option<&str>) -> Result<()> {
    let mut cmd = if let Some(line) = command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", line]);
        cmd
    } else {
        let mut cmd = Command::new(WL_COPY);
        cmd.args(["--type", mime]);
        cmd
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not run the copy command")?;
    child
        .stdin
        .take()
        .context("copy command has no stdin")?
        .write_all(data)
        .context("could not write to the copy command")?;
    if !child
        .wait()
        .context("copy command did not finish")?
        .success()
    {
        bail!("copy command failed");
    }
    Ok(())
}

struct Now {
    year: String,
    month: String,
    day: String,
    hour: String,
    min: String,
    sec: String,
}

fn now() -> Result<Now> {
    let out = stdout_of("date", &[DATE_FIELDS])?;
    let text = String::from_utf8_lossy(&out);
    let mut fields = text.split_whitespace().map(str::to_owned);
    let mut next = || fields.next().context("unexpected date output");
    Ok(Now {
        year: next()?,
        month: next()?,
        day: next()?,
        hour: next()?,
        min: next()?,
        sec: next()?,
    })
}

// the common strftime specifiers; anything else is kept as written
fn format_time(template: &str, now: &Now) -> String {
    let mut out = String::new();
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&now.year),
            Some('m') => out.push_str(&now.month),
            Some('d') => out.push_str(&now.day),
            Some('H') => out.push_str(&now.hour),
            Some('M') => out.push_str(&now.min),
            Some('S') => out.push_str(&now.sec),
            Some('F') => out.push_str(&format_time("%Y-%m-%d", now)),
            Some('T') => out.push_str(&format_time("%H:%M:%S", now)),
            Some('%') | None => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_owned(),
        None => PathBuf::from(path),
    }
}

fn resolve(template: &str) -> Result<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    Ok(expand_home(&format_time(template, &now()?), &home))
}

// name offered by the save-as dialog; stdout has no name, so use the default
pub fn suggested_path(template: &str) -> Result<PathBuf> {
    let template = if template == STDIO_PATH {
        DEFAULT_OUTPUT_FILENAME
    } else {
        template
    };
    resolve(template)
}

pub fn save(png: &[u8], template: &str) -> Result<PathBuf> {
    let path = resolve(template)?;
    save_to(png, &path)?;
    Ok(path)
}

// Copy of the image for the notification icon. The name is new every time and the file is
// created exclusively, so an existing path or symlink is never followed.
pub fn write_thumbnail(png: &[u8]) -> Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = thumbnail_dir();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{THUMBNAIL_PREFIX}-{}-{n}.png", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(&path)
        .with_context(|| format!("could not create {}", path.display()))?;
    file.write_all(png)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

fn thumbnail_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

// Thumbnails of earlier runs are never cleaned up by us, so sweep the old ones at startup.
pub fn remove_stale_thumbnails() {
    remove_stale(
        &thumbnail_dir(),
        Duration::from_secs(THUMBNAIL_MAX_AGE_SECS),
    );
}

fn remove_stale(dir: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let ours = entry
            .file_name()
            .to_string_lossy()
            .starts_with(THUMBNAIL_PREFIX);
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if ours && age.is_some_and(|age| age > max_age) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

pub fn save_to(png: &[u8], path: &Path) -> Result<()> {
    create_parent(path)?;
    fs::write(path, png).with_context(|| format!("could not write {}", path.display()))
}

fn create_parent(path: &Path) -> Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_time() -> Now {
        let field = |s: &str| s.to_owned();
        Now {
            year: field("2026"),
            month: field("03"),
            day: field("07"),
            hour: field("09"),
            min: field("05"),
            sec: field("01"),
        }
    }

    #[test]
    fn crop_has_exact_physical_size() {
        let mut image = Pixmap::new(1920, 1080).unwrap();
        image.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let png = crop_png(&image, IntRect::from_xywh(101, 53, 333, 217).unwrap()).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (333, 217));
        assert_eq!(decoded.pixel(0, 0).unwrap().red(), 10);
    }

    #[test]
    fn strftime_specifiers() {
        let now = sample_time();
        assert_eq!(
            format_time("shot-%Y%m%d-%H%M%S.png", &now),
            "shot-20260307-090501.png"
        );
        assert_eq!(
            format_time("%F_%T 100%% %q %", &now),
            "2026-03-07_09:05:01 100% %q %"
        );
    }

    #[test]
    fn only_old_thumbnails_are_removed() {
        let dir = std::env::temp_dir().join(format!("shotr-sweep-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let old = dir.join(format!("{THUMBNAIL_PREFIX}-1-0.png"));
        let fresh = dir.join(format!("{THUMBNAIL_PREFIX}-2-0.png"));
        let other = dir.join("unrelated.png");
        for path in [&old, &fresh, &other] {
            fs::write(path, b"x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        for path in [&old, &other] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(long_ago)
                .unwrap();
        }
        remove_stale(&dir, Duration::from_secs(600));
        assert!(!old.exists() && fresh.exists() && other.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/home/me");
        assert_eq!(
            expand_home("~/a/b.png", home),
            Path::new("/home/me/a/b.png")
        );
        assert_eq!(expand_home("~", home), home);
        assert_eq!(expand_home("/tmp/~x", home), Path::new("/tmp/~x"));
    }
}
