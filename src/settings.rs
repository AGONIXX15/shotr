use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, ValueEnum};
use serde::Deserialize;

use crate::{
    config::{
        COLOR_KEYS, CONFIG_DIR, CONFIG_FILE, CORNER_ROUNDNESS, DEFAULT_OUTPUT_FILENAME,
        DEFAULT_ROUND_CAPS, DEFAULT_SMOOTHING, FACTOR_DEFAULT, FACTOR_MAX, FACTOR_MIN, NAMESPACE,
        PALETTE, PAN_DEFAULT, TEXT_MOVE_DEFAULT, ZOOM_DEFAULT,
    },
    docs::{LICENSE, MAN_PAGE},
    style::Color,
    tools::{TOOL_KEYS, Tool},
    util,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Act {
    SaveToClipboard,
    SaveToFile,
    SaveToFileAs,
    CopyFilepathToClipboard,
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ExitTrigger {
    Copy,
    Save,
    SaveAs,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Highlighter {
    #[default]
    Block,
    Freehand,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Thumbnail {
    Screenshot,
    AppIcon,
}

// satty window options: accepted so its command lines work, but a fullscreen overlay has no use for them
#[derive(Args)]
#[allow(dead_code)]
struct WindowFlags {
    /// Ignored, shotr is always a fullscreen overlay
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    fullscreen: Option<String>,
    /// Ignored, shotr is always a fullscreen overlay
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    resize: Option<String>,
    /// Ignored, there is no window to float
    #[arg(long)]
    floating_hack: bool,
    /// Ignored, there is no window decoration
    #[arg(long)]
    no_window_decoration: bool,
    /// Ignored, there is no window title
    #[arg(long)]
    title: Option<String>,
    /// Ignored, there is no window app id
    #[arg(long)]
    app_id: Option<String>,
    /// Ignored, the overlay always uses the physical pixels
    #[arg(long)]
    input_scale: Option<f32>,
}

#[derive(Parser)]
#[command(name = "shotr", version, about = "Screenshot annotator for Hyprland")]
#[allow(clippy::struct_excessive_bools)] // one flag per independent switch
struct Cli {
    /// Print the manual page in roff format
    #[arg(long)]
    man: bool,
    /// Print the license
    #[arg(long)]
    license: bool,
    /// Print how long each startup stage takes
    #[arg(long)]
    profile_startup: bool,
    /// Copy to the clipboard after every annotation change
    #[arg(long)]
    auto_copy: bool,
    /// Save to a file as well after copying
    #[arg(long)]
    save_after_copy: bool,
    /// Deprecated, same as --actions-on-right-click save-to-clipboard
    #[arg(long)]
    right_click_copy: bool,
    /// Deprecated, use --actions-on-enter
    #[arg(long)]
    action_on_enter: Option<Act>,
    /// Show the toolbars while the overlay has keyboard focus
    #[arg(long)]
    focus_toggles_toolbars: bool,
    /// Points averaged for brush smoothing, 0 turns it off
    #[arg(long)]
    brush_smooth_history_size: Option<usize>,
    /// Zoom step of Ctrl+wheel
    #[arg(long)]
    zoom_factor: Option<f32>,
    /// Pixels moved by Alt+arrows
    #[arg(long)]
    pan_step_size: Option<f32>,
    /// Pixels a selected shape moves with the arrow keys
    #[arg(long)]
    text_move_length: Option<f32>,
    /// Icon of the notifications
    #[arg(long)]
    notification_thumbnail: Option<Thumbnail>,
    #[arg(long)]
    font_style: Option<String>,
    #[command(flatten)]
    window: WindowFlags,
    /// Config file, default is ~/.config/shotr/config.toml
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Image to annotate, or '-' for stdin; omit to capture the focused monitor
    #[arg(short, long)]
    filename: Option<String>,
    /// File to save to, may contain strftime specifiers and ~
    #[arg(short, long)]
    output_filename: Option<String>,
    /// Exit after these actions, no value means all
    #[arg(long, num_args = 0.., value_delimiter = ',')]
    early_exit: Option<Vec<ExitTrigger>>,
    #[arg(long, alias = "init-tool")]
    initial_tool: Option<Tool>,
    /// Command that receives the PNG on stdin, default is wl-copy
    #[arg(long)]
    copy_command: Option<String>,
    #[arg(long)]
    annotation_size_factor: Option<f32>,
    #[arg(long, value_delimiter = ',')]
    actions_on_enter: Option<Vec<Act>>,
    #[arg(long, value_delimiter = ',')]
    actions_on_escape: Option<Vec<Act>>,
    #[arg(long, value_delimiter = ',')]
    actions_on_right_click: Option<Vec<Act>>,
    #[arg(long)]
    corner_roundness: Option<f32>,
    #[arg(short = 'd', long)]
    default_hide_toolbars: bool,
    #[arg(long)]
    default_fill_shapes: bool,
    #[arg(long)]
    primary_highlighter: Option<Highlighter>,
    #[arg(long)]
    disable_notifications: bool,
    #[arg(long)]
    font_family: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct FileConfig {
    general: General,
    font: FontSection,
    color_palette: PaletteSection,
    keybinds: Keybinds,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
#[allow(clippy::struct_excessive_bools)] // mirrors the config keys
struct General {
    early_exit: Option<EarlyExit>,
    initial_tool: Option<Tool>,
    copy_command: Option<String>,
    annotation_size_factor: Option<f32>,
    output_filename: Option<String>,
    actions_on_enter: Option<Vec<Act>>,
    actions_on_escape: Option<Vec<Act>>,
    actions_on_right_click: Option<Vec<Act>>,
    corner_roundness: Option<f32>,
    default_hide_toolbars: bool,
    default_fill_shapes: bool,
    primary_highlighter: Option<Highlighter>,
    disable_notifications: bool,
    auto_copy: bool,
    save_after_copy: bool,
    right_click_copy: bool,
    action_on_enter: Option<Act>,
    focus_toggles_toolbars: bool,
    brush_smooth_history_size: Option<usize>,
    zoom_factor: Option<f32>,
    pan_step_size: Option<f32>,
    text_move_length: Option<f32>,
    notification_thumbnail: Option<Thumbnail>,
    default_round_caps: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EarlyExit {
    Flag(bool),
    List(Vec<ExitTrigger>),
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Keybinds {
    pointer: Option<String>,
    crop: Option<String>,
    brush: Option<String>,
    line: Option<String>,
    arrow: Option<String>,
    rectangle: Option<String>,
    ellipse: Option<String>,
    text: Option<String>,
    marker: Option<String>,
    blur: Option<String>,
    highlight: Option<String>,
}

impl Keybinds {
    // in `TOOL_KEYS` order
    fn overrides(&self) -> [(&'static str, Option<&str>); TOOL_KEYS.len()] {
        [
            ("pointer", self.pointer.as_deref()),
            ("crop", self.crop.as_deref()),
            ("brush", self.brush.as_deref()),
            ("line", self.line.as_deref()),
            ("arrow", self.arrow.as_deref()),
            ("rectangle", self.rectangle.as_deref()),
            ("ellipse", self.ellipse.as_deref()),
            ("text", self.text.as_deref()),
            ("marker", self.marker.as_deref()),
            ("blur", self.blur.as_deref()),
            ("highlight", self.highlight.as_deref()),
        ]
    }

    fn resolve(&self) -> Result<[(char, Tool); TOOL_KEYS.len()]> {
        let mut keys = TOOL_KEYS;
        for (slot, (name, custom)) in keys.iter_mut().zip(self.overrides()) {
            let Some(text) = custom else { continue };
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(key), None) => slot.0 = key,
                _ => bail!("keybinds.{name} must be a single character, got {text:?}"),
            }
        }
        for (i, (key, tool)) in keys.iter().enumerate() {
            if keys[..i].iter().any(|(other, _)| other == key) {
                bail!("key {key:?} is bound to more than one tool, again for {tool:?}");
            }
        }
        Ok(keys)
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FontSection {
    family: Option<String>,
    style: Option<String>,
    fallback: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PaletteSection {
    palette: Option<Vec<String>>,
}

#[allow(clippy::struct_excessive_bools)] // independent on/off settings
pub struct Settings {
    pub input: Option<String>,
    pub early_exit: Vec<ExitTrigger>,
    pub initial_tool: Tool,
    pub copy_command: Option<String>,
    pub size_factor: f32,
    pub output_filename: String,
    pub on_enter: Vec<Act>,
    pub on_escape: Vec<Act>,
    pub on_right_click: Vec<Act>,
    pub corner_roundness: f32,
    pub hide_toolbars: bool,
    pub fill_shapes: bool,
    pub highlighter: Highlighter,
    pub notifications: bool,
    pub font_family: Option<String>,
    pub palette: Vec<Color>,
    pub tool_keys: [(char, Tool); TOOL_KEYS.len()],
    pub font_style: Option<String>,
    pub font_fallback: Vec<String>,
    pub auto_copy: bool,
    pub save_after_copy: bool,
    pub focus_toggles_toolbars: bool,
    pub profile_startup: bool,
    pub smoothing: usize,
    pub zoom_factor: f32,
    pub pan_step: f32,
    pub text_move: f32,
    pub thumbnail: Option<Thumbnail>,
    pub round_caps: bool,
}

pub fn exits_after(early_exit: &[ExitTrigger], act: Act) -> bool {
    let trigger = match act {
        Act::Exit => return true,
        Act::SaveToClipboard | Act::CopyFilepathToClipboard => ExitTrigger::Copy,
        Act::SaveToFile => ExitTrigger::Save,
        Act::SaveToFileAs => ExitTrigger::SaveAs,
    };
    early_exit
        .iter()
        .any(|t| *t == trigger || *t == ExitTrigger::All)
}

impl Settings {
    #[cfg(test)]
    pub fn for_tests() -> Result<Self> {
        Self::resolve(Cli::parse_from([NAMESPACE]), FileConfig::default())
    }

    // None when --man or --license already printed what was asked for
    pub fn load() -> Result<Option<Self>> {
        let cli = Cli::parse();
        if cli.man {
            print!("{MAN_PAGE}");
            return Ok(None);
        }
        if cli.license {
            print!("{LICENSE}");
            return Ok(None);
        }
        let file = read_config(cli.config.as_deref())?;
        Self::resolve(cli, file).map(Some)
    }

    fn resolve(cli: Cli, file: FileConfig) -> Result<Self> {
        let FileConfig {
            general: g,
            font,
            color_palette,
            keybinds,
        } = file;
        let early_exit = match (cli.early_exit, g.early_exit) {
            (Some(list), _) if list.is_empty() => vec![ExitTrigger::All],
            (Some(list), _) | (None, Some(EarlyExit::List(list))) => list,
            (None, Some(EarlyExit::Flag(true))) => vec![ExitTrigger::All],
            (None, _) => Vec::new(),
        };
        let mut palette = match color_palette.palette {
            Some(list) => list
                .iter()
                .map(|s| Color::parse(s).ok_or_else(|| anyhow!("invalid palette colour {s}")))
                .collect::<Result<Vec<_>>>()?,
            None => PALETTE.to_vec(),
        };
        if palette.is_empty() {
            return Err(anyhow!("the colour palette is empty"));
        }
        if palette.len() > COLOR_KEYS.len() {
            let message = format!("palette truncated to {} colours", COLOR_KEYS.len());
            util::notify(false, NAMESPACE, &message, None);
            palette.truncate(COLOR_KEYS.len());
        }
        let corner_roundness = cli
            .corner_roundness
            .or(g.corner_roundness)
            .unwrap_or(CORNER_ROUNDNESS);
        if corner_roundness < 0.0 {
            return Err(anyhow!("corner-roundness must not be negative"));
        }
        let size_factor = cli
            .annotation_size_factor
            .or(g.annotation_size_factor)
            .unwrap_or(FACTOR_DEFAULT)
            .clamp(FACTOR_MIN, FACTOR_MAX);
        let zoom_factor = cli.zoom_factor.or(g.zoom_factor).unwrap_or(ZOOM_DEFAULT);
        if zoom_factor <= 0.0 {
            return Err(anyhow!("zoom-factor must be positive"));
        }
        // the command line wins over the file, and within each a list wins over the old alias
        let copy_action = || vec![Act::SaveToClipboard];
        let on_enter = cli
            .actions_on_enter
            .or_else(|| cli.action_on_enter.map(|a| vec![a]))
            .or(g.actions_on_enter)
            .or_else(|| g.action_on_enter.map(|a| vec![a]));
        let on_right_click = cli
            .actions_on_right_click
            .or_else(|| cli.right_click_copy.then(copy_action))
            .or(g.actions_on_right_click)
            .or_else(|| g.right_click_copy.then(copy_action));
        Ok(Self {
            input: cli.filename,
            early_exit,
            initial_tool: cli.initial_tool.or(g.initial_tool).unwrap_or(Tool::DEFAULT),
            copy_command: cli.copy_command.or(g.copy_command),
            size_factor,
            output_filename: cli
                .output_filename
                .or(g.output_filename)
                .unwrap_or_else(|| DEFAULT_OUTPUT_FILENAME.to_owned()),
            on_enter: on_enter.unwrap_or_else(copy_action),
            on_escape: cli
                .actions_on_escape
                .or(g.actions_on_escape)
                .unwrap_or_else(|| vec![Act::Exit]),
            on_right_click: on_right_click.unwrap_or_default(),
            corner_roundness,
            hide_toolbars: cli.default_hide_toolbars || g.default_hide_toolbars,
            fill_shapes: cli.default_fill_shapes || g.default_fill_shapes,
            highlighter: cli
                .primary_highlighter
                .or(g.primary_highlighter)
                .unwrap_or_default(),
            notifications: !(cli.disable_notifications || g.disable_notifications),
            font_family: cli.font_family.or(font.family),
            font_style: cli.font_style.or(font.style),
            font_fallback: font.fallback,
            palette,
            tool_keys: keybinds.resolve()?,
            auto_copy: cli.auto_copy || g.auto_copy,
            save_after_copy: cli.save_after_copy || g.save_after_copy,
            focus_toggles_toolbars: cli.focus_toggles_toolbars || g.focus_toggles_toolbars,
            profile_startup: cli.profile_startup,
            smoothing: cli
                .brush_smooth_history_size
                .or(g.brush_smooth_history_size)
                .unwrap_or(DEFAULT_SMOOTHING),
            zoom_factor,
            pan_step: cli.pan_step_size.or(g.pan_step_size).unwrap_or(PAN_DEFAULT),
            text_move: cli
                .text_move_length
                .or(g.text_move_length)
                .unwrap_or(TEXT_MOVE_DEFAULT),
            thumbnail: cli.notification_thumbnail.or(g.notification_thumbnail),
            round_caps: g.default_round_caps.unwrap_or(DEFAULT_ROUND_CAPS),
        })
    }
}

fn read_config(explicit: Option<&std::path::Path>) -> Result<FileConfig> {
    let path = if let Some(path) = explicit {
        path.to_owned()
    } else {
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => {
                PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config")
            }
        };
        base.join(CONFIG_DIR).join(CONFIG_FILE)
    };
    match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("invalid {}", path.display())),
        Err(e) if explicit.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
            Ok(FileConfig::default())
        }
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(args: &[&str], toml: &str) -> Settings {
        let cli = Cli::parse_from(std::iter::once("shotr").chain(args.iter().copied()));
        Settings::resolve(cli, toml::from_str(toml).unwrap()).unwrap()
    }

    #[test]
    fn defaults_without_cli_or_file() {
        let s = settings(&[], "");
        assert_eq!(s.on_enter, [Act::SaveToClipboard]);
        assert_eq!(s.on_escape, [Act::Exit]);
        assert_eq!(s.on_right_click, []);
        assert!(s.early_exit.is_empty() && s.notifications);
        assert_eq!(s.palette.len(), PALETTE.len());
    }

    #[test]
    fn cli_overrides_file_which_overrides_default() {
        let toml =
            "[general]\ncorner-roundness = 4\ninitial-tool = \"brush\"\ncopy-command = \"xclip\"";
        let s = settings(
            &["--initial-tool", "rectangle", "--corner-roundness", "0"],
            toml,
        );
        assert_eq!(s.initial_tool, Tool::Rect);
        assert!(s.corner_roundness.abs() < f32::EPSILON);
        assert_eq!(s.copy_command.as_deref(), Some("xclip"));
    }

    #[test]
    fn early_exit_forms() {
        assert!(exits_after(
            &settings(&["--early-exit"], "").early_exit,
            Act::SaveToFile
        ));
        assert!(exits_after(
            &settings(&[], "[general]\nearly-exit = true").early_exit,
            Act::SaveToFile
        ));
        let only_save = settings(&["--early-exit", "save"], "");
        assert!(exits_after(&only_save.early_exit, Act::SaveToFile));
        assert!(!exits_after(&only_save.early_exit, Act::SaveToClipboard));
        assert!(!exits_after(&only_save.early_exit, Act::SaveToFileAs));
        assert!(exits_after(&only_save.early_exit, Act::Exit));
        assert!(!exits_after(
            &settings(&[], "[general]\nearly-exit = false").early_exit,
            Act::SaveToFile
        ));
    }

    #[test]
    fn palette_and_actions_from_file() {
        let toml = "[color-palette]\npalette = [\"#ff0000ff\", \"#00ff00\"]\n[general]\nactions-on-enter = [\"save-to-file\", \"exit\"]";
        let s = settings(&[], toml);
        assert_eq!(s.palette, [Color::rgb(255, 0, 0), Color::rgb(0, 255, 0)]);
        assert_eq!(s.on_enter, [Act::SaveToFile, Act::Exit]);
    }

    #[test]
    fn bad_palette_is_an_error() {
        let cli = Cli::parse_from(["shotr"]);
        let file: FileConfig = toml::from_str("[color-palette]\npalette = [\"red\"]").unwrap();
        assert!(Settings::resolve(cli, file).is_err());
    }

    #[test]
    fn size_factor_is_clamped() {
        assert!(
            (settings(&["--annotation-size-factor", "0"], "").size_factor - FACTOR_MIN).abs()
                < f32::EPSILON
        );
        assert!(
            (settings(&["--annotation-size-factor", "500"], "").size_factor - FACTOR_MAX).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn negative_roundness_is_rejected() {
        let cli = Cli::parse_from(["shotr", "--corner-roundness=-1"]);
        assert!(Settings::resolve(cli, FileConfig::default()).is_err());
    }

    #[test]
    fn palette_is_capped_to_the_number_keys() {
        let colors = vec!["\"#112233\""; COLOR_KEYS.len() + 3].join(",");
        let s = settings(&[], &format!("[color-palette]\npalette = [{colors}]"));
        assert_eq!(s.palette.len(), COLOR_KEYS.len());
    }

    #[test]
    fn keybinds_override_defaults() {
        let s = settings(&[], "[keybinds]\nblur = \"x\"\narrow = \"a\"");
        let key_of = |tool| {
            s.tool_keys
                .iter()
                .find(|(_, t)| *t == tool)
                .map(|(k, _)| *k)
        };
        assert_eq!(key_of(Tool::Blur), Some('x'));
        assert_eq!(key_of(Tool::Arrow), Some('a'));
        assert_eq!(key_of(Tool::Pointer), Some('p'));
    }

    #[test]
    fn bad_keybinds_are_errors() {
        for toml in [
            "[keybinds]\nblur = \"xy\"",
            "[keybinds]\nblur = \"\"",
            "[keybinds]\nblur = \"p\"",
        ] {
            let cli = Cli::parse_from(["shotr"]);
            let file: FileConfig = toml::from_str(toml).unwrap();
            assert!(Settings::resolve(cli, file).is_err(), "{toml}");
        }
    }

    #[test]
    fn deprecated_flags_map_to_action_lists() {
        let s = settings(
            &["--right-click-copy", "--action-on-enter", "save-to-file"],
            "",
        );
        assert_eq!(s.on_right_click, [Act::SaveToClipboard]);
        assert_eq!(s.on_enter, [Act::SaveToFile]);
        let s = settings(
            &[],
            "[general]\nright-click-copy = true\naction-on-enter = \"save-to-file\"",
        );
        assert_eq!(
            (s.on_right_click, s.on_enter),
            (vec![Act::SaveToClipboard], vec![Act::SaveToFile])
        );
        let s = settings(
            &[
                "--action-on-enter",
                "save-to-file",
                "--actions-on-enter",
                "exit",
            ],
            "",
        );
        assert_eq!(s.on_enter, [Act::Exit], "the list wins over the alias");
    }

    #[test]
    fn window_flags_are_accepted_and_ignored() {
        let args = [
            "--fullscreen",
            "--resize",
            "--floating-hack",
            "--no-window-decoration",
            "--title",
            "x",
            "--app-id",
            "a.b",
            "--input-scale",
            "2",
        ];
        assert_eq!(settings(&args, "").on_escape, [Act::Exit]);
        let args = ["--fullscreen", "all", "--resize", "800x600", "-f", "-"];
        assert_eq!(settings(&args, "").input.as_deref(), Some("-"));
    }

    #[test]
    fn behaviour_flags_and_defaults() {
        let s = settings(&[], "");
        assert!((s.zoom_factor - ZOOM_DEFAULT).abs() < f32::EPSILON);
        assert!((s.pan_step - PAN_DEFAULT).abs() < f32::EPSILON);
        assert_eq!(s.smoothing, DEFAULT_SMOOTHING);
        assert!(!s.auto_copy && !s.save_after_copy && s.thumbnail.is_none());
        let args = [
            "--auto-copy",
            "--save-after-copy",
            "--brush-smooth-history-size",
            "5",
            "--notification-thumbnail",
            "app-icon",
            "--focus-toggles-toolbars",
            "--profile-startup",
        ];
        let s = settings(&args, "[general]\nzoom-factor = 1.5");
        assert!(s.auto_copy && s.save_after_copy && s.focus_toggles_toolbars && s.profile_startup);
        assert_eq!((s.smoothing, s.thumbnail), (5, Some(Thumbnail::AppIcon)));
        assert!((s.zoom_factor - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn zoom_factor_must_be_positive() {
        let cli = Cli::parse_from(["shotr", "--zoom-factor", "0"]);
        assert!(Settings::resolve(cli, FileConfig::default()).is_err());
    }

    #[test]
    fn round_caps_come_from_the_config() {
        assert_eq!(settings(&[], "").round_caps, DEFAULT_ROUND_CAPS);
        assert!(!settings(&[], "[general]\ndefault-round-caps = false").round_caps);
    }

    #[test]
    fn font_fallback_list_is_read() {
        let s = settings(
            &[],
            "[font]\nfamily = \"Roboto\"\nfallback = [\"Noto Sans CJK SC\", \"IPAGothic\"]",
        );
        assert_eq!(s.font_fallback, ["Noto Sans CJK SC", "IPAGothic"]);
        assert_eq!(settings(&[], "").font_fallback, Vec::<String>::new());
    }

    #[test]
    fn version_flag_exists() {
        let err = Cli::try_parse_from(["shotr", "--version"]).err().unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn command_line_alias_beats_a_file_list() {
        let toml = "[general]\nactions-on-enter = [\"exit\"]\nactions-on-right-click = [\"exit\"]";
        let s = settings(
            &["--action-on-enter", "save-to-file", "--right-click-copy"],
            toml,
        );
        assert_eq!(s.on_enter, [Act::SaveToFile]);
        assert_eq!(s.on_right_click, [Act::SaveToClipboard]);
    }
}
