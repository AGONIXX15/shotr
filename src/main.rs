// pixel math on screen-sized values; a crate attribute because `-W clippy::pedantic`
// on the command line overrides the [lints] table in Cargo.toml
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

mod config;
mod docs;
mod editor;
mod font;
mod history;
mod output;
mod overlay;
mod settings;
mod style;
mod text;
mod tools;
mod ui;
mod util;
mod view;

use std::{io::Read, process::ExitCode};

use anyhow::{Context, Result};
use tiny_skia::Pixmap;

use config::{GRIM_ARGS, MSG_FAILED, STDIO_PATH};
use font::Fonts;
use settings::Settings;
use util::{Stopwatch, stdout_of};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            util::notify(true, MSG_FAILED, &format!("{e:#}"), None);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let Some(settings) = Settings::load()? else {
        return Ok(());
    };
    let stopwatch = Stopwatch::new(settings.profile_startup);
    output::remove_stale_thumbnails();
    let (frame, monitor) = if let Some(path) = settings.input.as_deref() {
        // a loaded image still opens on the focused monitor, if hyprctl can tell
        (read_image(path)?, focused_monitor().ok())
    } else {
        let monitor = focused_monitor()?;
        (capture(&monitor)?, Some(monitor))
    };
    stopwatch.mark("capture");
    let fonts = Fonts::load(
        settings.font_family.as_deref(),
        settings.font_style.as_deref(),
        &settings.font_fallback,
    )?;
    stopwatch.mark("fonts");
    overlay::run(monitor.as_deref(), frame, fonts, settings, stopwatch)
}

fn read_image(path: &str) -> Result<Pixmap> {
    let bytes = if path == STDIO_PATH {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("could not read stdin")?;
        bytes
    } else {
        std::fs::read(path).with_context(|| format!("could not read {path}"))?
    };
    Pixmap::decode_png(&bytes).context("could not decode the image, only PNG is supported")
}

// shortcut: string search instead of a JSON parser, use serde_json if hyprctl's output changes
fn json_str_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let after = json.split_once(&format!("\"{key}\""))?.1;
    let value = after
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    value.split_once('"').map(|(v, _)| v)
}

fn focused_monitor() -> Result<String> {
    let workspace = stdout_of("hyprctl", &["activeworkspace", "-j"])?;
    let workspace = String::from_utf8_lossy(&workspace);
    let monitor = json_str_field(&workspace, "monitor").context("hyprctl returned no monitor")?;
    Ok(monitor.to_owned())
}

fn capture(monitor: &str) -> Result<Pixmap> {
    let args = [&GRIM_ARGS[..], &["-o", monitor, "-"]].concat();
    let png = stdout_of("grim", &args)?;
    Pixmap::decode_png(&png).context("could not decode grim output")
}

#[cfg(test)]
mod tests {
    use super::json_str_field;

    #[test]
    fn finds_monitor_in_hyprctl_json() {
        let json = "{\n    \"id\": 1,\n    \"monitor\": \"eDP-1\",\n    \"windows\": 2\n}";
        assert_eq!(json_str_field(json, "monitor"), Some("eDP-1"));
        assert_eq!(json_str_field(json, "nope"), None);
    }

    #[test]
    fn null_or_non_string_value_is_none() {
        assert_eq!(
            json_str_field("{\"monitor\": null, \"x\": \"y\"}", "monitor"),
            None
        );
        assert_eq!(json_str_field("{\"monitor\": 3}", "monitor"), None);
    }
}
