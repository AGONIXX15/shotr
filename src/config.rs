use crate::style::Color;

pub const NAMESPACE: &str = "shotr";
pub const FONT_QUERY: &str = "sans:style=Regular:fontformat=TrueType";
pub const DEFAULT_FONT_FAMILY: &str = "sans";
pub const ICON_FONT_QUERY: &str = "JetBrainsMono Nerd Font";
// fc-match always answers, only accept a real Nerd Font
pub const ICON_FONT_MARK: &str = "Nerd";
// compression costs ~300ms and the PNG never leaves this process
pub const GRIM_ARGS: [&str; 2] = ["-l", "0"];
pub const DEFAULT_OUTPUT_FILENAME: &str = "~/Pictures/Screenshots/shot-%Y%m%d-%H%M%S.png";
pub const STDIO_PATH: &str = "-";
pub const WL_COPY: &str = "wl-copy";
pub const PNG_MIME: &str = "image/png";
pub const TEXT_MIME: &str = "text/plain";
pub const CONFIG_DIR: &str = "shotr";
pub const CONFIG_FILE: &str = "config.toml";

pub const BYTES_PER_PIXEL: usize = 4;
pub const RGB_CHANNELS: usize = 3;

// linux/input-event-codes.h
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
// evdev codes of the Latin shortcut letters, so Ctrl+C works on any layout
pub const LATIN_KEYS: [(u32, char); 5] = [(46, 'c'), (31, 's'), (44, 'z'), (21, 'y'), (20, 't')];

pub const DIM_ALPHA: u8 = 140;
pub const BORDER_COLOR: Color = Color::rgb(255, 255, 255);
pub const BORDER_WIDTH: f32 = 2.0;
pub const HANDLE_SIZE: f32 = 14.0;
pub const HANDLE_HIT: f32 = 14.0;
pub const SELECT_COLOR: Color = Color::rgb(80, 160, 255);
pub const SELECT_PAD: f32 = 6.0;
pub const SELECT_WIDTH: f32 = 2.0;
pub const HIT_TOLERANCE: f32 = 8.0;
pub const VIEW_BACKGROUND: Color = Color::rgb(24, 24, 28);

pub const PALETTE: [Color; 10] = [
    Color::rgb(230, 40, 40),
    Color::rgb(250, 140, 20),
    Color::rgb(250, 200, 20),
    Color::rgb(60, 190, 80),
    Color::rgb(30, 190, 190),
    Color::rgb(50, 120, 240),
    Color::rgb(150, 80, 220),
    Color::rgb(240, 90, 170),
    Color::rgb(255, 255, 255),
    Color::rgb(20, 20, 20),
];
pub const COLOR_KEYS: [char; 10] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];

pub const SIZES: [f32; 3] = [2.0, 4.0, 8.0];
pub const SIZE_LABELS: [&str; SIZES.len()] = ["S", "M", "L"];
pub const DEFAULT_SIZE: usize = 1;
pub const FACTOR_DEFAULT: f32 = 1.0;
pub const FACTOR_MIN: f32 = 0.1;
pub const FACTOR_MAX: f32 = 99.99;
pub const FACTOR_STEP: f32 = 0.1;
pub const FACTOR_FINE_STEP: f32 = 0.01;
pub const FACTOR_COARSE_STEP: f32 = 1.0;

pub const ZOOM_DEFAULT: f32 = 1.1;
pub const ZOOM_MAX: f32 = 16.0;
pub const PAN_DEFAULT: f32 = 50.0;
pub const TEXT_MOVE_DEFAULT: f32 = 50.0;
pub const SCROLL_PAN: f32 = 1.0;

pub const MIN_DRAG: f32 = 3.0;
pub const SNAP_STEP_DEG: f32 = 15.0;
pub const ARROW_HEAD_PER_WIDTH: f32 = 4.0;
pub const ARROW_HEAD_ANGLE: f32 = 0.5;
pub const CORNER_ROUNDNESS: f32 = 12.0;
pub const BLUR_RADIUS: usize = 12;
pub const TEXT_PX_PER_WIDTH: f32 = 8.0;
pub const CARET_WIDTH: f32 = 2.0;
pub const MARKER_RADIUS_PER_WIDTH: f32 = 4.0;
pub const MARKER_TEXT_RATIO: f32 = 1.2;
pub const MARKER_TEXT_COLOR: Color = Color::rgb(255, 255, 255);
pub const HIGHLIGHT_ALPHA: u8 = 110;
pub const HIGHLIGHT_PEN_FACTOR: f32 = 4.0;

pub const BUTTON_SIZE: f32 = 48.0;
pub const BUTTON_GAP: f32 = 6.0;
pub const BAR_PADDING: f32 = 8.0;
pub const BAR_MARGIN: f32 = 12.0;
pub const BAR_RADIUS: f32 = 12.0;
pub const BAR_COLOR: Color = Color::rgb(30, 30, 34).with_alpha(235);
pub const BUTTON_ACTIVE_COLOR: Color = Color::rgb(70, 110, 190);
pub const BUTTON_TEXT_COLOR: Color = Color::rgb(240, 240, 240);
pub const BUTTON_RADIUS: f32 = 8.0;
pub const ICON_PX: f32 = 26.0;
pub const LABEL_PX: f32 = 18.0;
pub const SWATCH_INSET: f32 = 8.0;

// Font Awesome codepoints as shipped in Nerd Fonts, in `Tool` declaration order
pub const TOOL_ICONS: [char; 11] = [
    '\u{f245}', '\u{f125}', '\u{f1fc}', '\u{f068}', '\u{f178}', '\u{f096}', '\u{f10c}', '\u{f031}',
    '\u{f292}', '\u{f043}', '\u{f040}',
];
pub const TOOL_LABELS: [&str; 11] = [
    "Sel", "Crop", "Pen", "Line", "Arw", "Rect", "Oval", "Txt", "#", "Blur", "Hi",
];
pub const ICON_UNDO: char = '\u{f0e2}';
pub const ICON_REDO: char = '\u{f01e}';
pub const ICON_COPY: char = '\u{f0c5}';
pub const ICON_SAVE: char = '\u{f0c7}';
pub const ICON_CUSTOM_COLOR: char = '\u{f1fb}';
pub const ICON_FILL: char = '\u{f0c8}';
pub const LABEL_UNDO: &str = "Undo";
pub const LABEL_REDO: &str = "Redo";
pub const LABEL_COPY: &str = "Copy";
pub const LABEL_SAVE: &str = "Save";
pub const LABEL_CUSTOM_COLOR: &str = "Col";
pub const LABEL_FILL: &str = "Fill";
pub const LABEL_FACTOR_DOWN: &str = "-";
pub const LABEL_FACTOR_UP: &str = "+";

pub const ZENITY_COLOR_ARGS: [&str; 1] = ["--color-selection"];
pub const ZENITY_SAVE_ARGS: [&str; 3] = ["--file-selection", "--save", "--confirm-overwrite"];

pub const MSG_COPIED: &str = "Copied";
pub const MSG_SAVED: &str = "Saved";
pub const MSG_PATH_COPIED: &str = "Path copied";
pub const MSG_FAILED: &str = "shotr failed";
pub const MSG_NO_OUTPUT: &str = "Monitor not found, the compositor picks the output";
pub const DATE_FIELDS: &str = "+%Y %m %d %H %M %S";
pub const OUTPUT_ROUNDTRIPS: usize = 2;
pub const APP_ICON: &str = "applets-screenshooter";
pub const THUMBNAIL_PREFIX: &str = "shotr-thumbnail";
// variable fonts carry the instance in the high bits of fontconfig's index
pub const FACE_INDEX_MASK: u32 = 0xFFFF;
pub const THUMBNAIL_MAX_AGE_SECS: u64 = 600;
// the text input protocol limits the surrounding text
pub const IME_SURROUNDING_MAX: usize = 4000;
pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const DEFAULT_SMOOTHING: usize = 0;
pub const DEFAULT_ROUND_CAPS: bool = true;
pub const PREEDIT_UNDERLINE_WIDTH: f32 = 2.0;
pub const UNDERLINE_OFFSET: f32 = 3.0;
pub const ZENITY_PROGRAM: &str = "zenity";
