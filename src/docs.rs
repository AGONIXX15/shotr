pub const LICENSE: &str = include_str!("../LICENSE");

pub const MAN_PAGE: &str = r".TH SHOTR 1
.SH NAME
shotr \- screenshot annotator for Hyprland
.SH SYNOPSIS
.B shotr
[\fIOPTIONS\fR]
.SH DESCRIPTION
Captures the focused monitor (or opens an existing PNG) in a fullscreen overlay, lets you
select a region and annotate it, then copies or saves the result. Drawing is done on the CPU,
no OpenGL is needed.
.PP
The satty window options (\-\-fullscreen, \-\-resize, \-\-floating\-hack,
\-\-no\-window\-decoration, \-\-title, \-\-app\-id, \-\-input\-scale) are accepted and ignored,
so satty command lines keep working. Run \fBshotr \-\-help\fR for every option.
.SH OPTIONS
.TP
\fB\-f\fR, \fB\-\-filename\fR \fIFILE\fR
PNG to annotate, or \- for stdin. Without it the focused monitor is captured.
.TP
\fB\-o\fR, \fB\-\-output\-filename\fR \fIFILE\fR
File for the save action. Accepts ~ and the strftime specifiers %Y %m %d %H %M %S %F %T.
.TP
\fB\-c\fR, \fB\-\-config\fR \fIFILE\fR
Config file, by default ~/.config/shotr/config.toml.
.TP
\fB\-\-early\-exit\fR [\fIall|copy|save|save\-as\fR]
Exit after these actions.
.TP
\fB\-\-actions\-on\-enter\fR, \fB\-\-actions\-on\-escape\fR, \fB\-\-actions\-on\-right\-click\fR
Comma separated list of save\-to\-clipboard, save\-to\-file, save\-to\-file\-as,
copy\-filepath\-to\-clipboard and exit.
.TP
\fB\-\-help\fR
Lists every option.
.SH KEYS
Tools: p pointer, c crop, b brush, i line, z arrow, r rectangle, e ellipse, t text,
m marker, u blur, g highlight. Ctrl+C copy, Ctrl+S save, Ctrl+Shift+S save as,
Ctrl+Alt+C copy the saved path, Ctrl+Z undo, Ctrl+Y redo, Ctrl+T toolbars.
.SH FILES
~/.config/shotr/config.toml
";
