use clap::ValueEnum;
use serde::Deserialize;
use tiny_skia::{
    FillRule, IntRect, LineCap, LineJoin, Path, PathBuilder, Pixmap, Point, Rect, Stroke, Transform,
};

use crate::{
    config::{
        ARROW_HEAD_ANGLE, ARROW_HEAD_PER_WIDTH, BLUR_RADIUS, BYTES_PER_PIXEL, HIGHLIGHT_ALPHA,
        HIGHLIGHT_PEN_FACTOR, MARKER_RADIUS_PER_WIDTH, MARKER_TEXT_COLOR, MARKER_TEXT_RATIO,
        MIN_DRAG, PREEDIT_UNDERLINE_WIDTH, RGB_CHANNELS, SNAP_STEP_DEG, TEXT_PX_PER_WIDTH,
        UNDERLINE_OFFSET,
    },
    font::Face,
    settings::Highlighter,
    style::{Color, OPAQUE, Style},
    text::TextBuf,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Pointer,
    Crop,
    Brush,
    Line,
    Arrow,
    #[serde(rename = "rectangle")]
    #[value(name = "rectangle")]
    Rect,
    Ellipse,
    Text,
    Marker,
    Blur,
    Highlight,
}

// default keys, kept here instead of config to avoid a config <-> tools cycle
pub const TOOL_KEYS: [(char, Tool); 11] = [
    ('p', Tool::Pointer),
    ('c', Tool::Crop),
    ('b', Tool::Brush),
    ('i', Tool::Line),
    ('z', Tool::Arrow),
    ('r', Tool::Rect),
    ('e', Tool::Ellipse),
    ('t', Tool::Text),
    ('m', Tool::Marker),
    ('u', Tool::Blur),
    ('g', Tool::Highlight),
];

impl Tool {
    pub const DEFAULT: Self = Self::Arrow;
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

pub struct Span {
    origin: Point,
    a: Point,
    b: Point,
}

impl Span {
    fn new(origin: Point) -> Self {
        Self {
            origin,
            a: origin,
            b: origin,
        }
    }

    fn drag_line(&mut self, to: Point, mods: Mods) {
        self.a = self.origin;
        self.b = if mods.shift {
            snap_angle(self.origin, to)
        } else {
            to
        };
    }

    fn drag_box(&mut self, to: Point, mods: Mods) {
        (self.a, self.b) = box_corners(self.origin, to, mods);
    }

    fn size(&self) -> f32 {
        (self.b.x - self.a.x).abs().max((self.b.y - self.a.y).abs())
    }

    fn rect(&self) -> Option<Rect> {
        Rect::from_ltrb(
            self.a.x.min(self.b.x),
            self.a.y.min(self.b.y),
            self.a.x.max(self.b.x),
            self.a.y.max(self.b.y),
        )
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        for p in [&mut self.origin, &mut self.a, &mut self.b] {
            shift(p, dx, dy);
        }
    }
}

pub enum Shape {
    Brush {
        points: Vec<Point>,
        style: Style,
    },
    Line {
        span: Span,
        style: Style,
    },
    Arrow {
        span: Span,
        style: Style,
    },
    Rect {
        span: Span,
        style: Style,
    },
    Ellipse {
        span: Span,
        style: Style,
    },
    Text {
        pos: Point,
        buf: TextBuf,
        style: Style,
    },
    Marker {
        pos: Point,
        number: u32,
        style: Style,
    },
    Blur {
        span: Span,
    },
    Highlight {
        span: Span,
        style: Style,
    },
    HighlightPen {
        points: Vec<Point>,
        style: Style,
    },
}

// what the active settings add to a new shape
#[derive(Clone, Copy)]
pub struct Extra {
    pub marker: u32,
    pub highlighter: Highlighter,
}

impl Shape {
    // None for the tools that don't create shapes (pointer, crop)
    pub fn start(tool: Tool, at: Point, style: Style, mods: Mods, extra: Extra) -> Option<Self> {
        let span = Span::new(at);
        Some(match tool {
            Tool::Pointer | Tool::Crop => return None,
            Tool::Brush => Self::Brush {
                points: vec![at],
                style,
            },
            Tool::Line => Self::Line { span, style },
            Tool::Arrow => Self::Arrow { span, style },
            Tool::Rect => Self::Rect { span, style },
            Tool::Ellipse => Self::Ellipse { span, style },
            Tool::Text => Self::Text {
                pos: at,
                buf: TextBuf::default(),
                style,
            },
            Tool::Marker => Self::Marker {
                pos: at,
                number: extra.marker,
                style,
            },
            Tool::Blur => Self::Blur { span },
            Tool::Highlight if (extra.highlighter == Highlighter::Freehand) != mods.ctrl => {
                Self::HighlightPen {
                    points: vec![at],
                    style,
                }
            }
            Tool::Highlight => Self::Highlight { span, style },
        })
    }

    // `smoothing` is the number of brush points averaged, 0 or 1 meaning raw input
    pub fn drag_to(&mut self, at: Point, mods: Mods, smoothing: usize) {
        match self {
            Self::Line { span, .. } | Self::Arrow { span, .. } => span.drag_line(at, mods),
            Self::Rect { span, .. }
            | Self::Ellipse { span, .. }
            | Self::Blur { span }
            | Self::Highlight { span, .. } => span.drag_box(at, mods),
            Self::Brush { points, .. } => {
                let next = smoothed(points, at, smoothing);
                points.push(next);
            }
            Self::HighlightPen { points, .. } => points.push(at),
            Self::Text { .. } | Self::Marker { .. } => {}
        }
    }

    pub fn text(&self) -> Option<&TextBuf> {
        match self {
            Self::Text { buf, .. } => Some(buf),
            _ => None,
        }
    }

    pub fn text_mut(&mut self) -> Option<&mut TextBuf> {
        match self {
            Self::Text { buf, .. } => Some(buf),
            _ => None,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    pub fn marker_number(&self) -> Option<u32> {
        match self {
            Self::Marker { number, .. } => Some(*number),
            _ => None,
        }
    }

    // empty text, or a click without a drag
    pub fn is_degenerate(&self) -> bool {
        match self {
            Self::Text { buf, .. } => buf.is_empty(),
            Self::HighlightPen { points, .. } => points.len() < 2,
            Self::Line { span, .. }
            | Self::Arrow { span, .. }
            | Self::Rect { span, .. }
            | Self::Ellipse { span, .. }
            | Self::Highlight { span, .. }
            | Self::Blur { span } => span.size() < MIN_DRAG,
            Self::Brush { .. } | Self::Marker { .. } => false,
        }
    }

    pub fn set_color(&mut self, color: Color) {
        match self {
            Self::Brush { style, .. }
            | Self::Line { style, .. }
            | Self::Arrow { style, .. }
            | Self::Rect { style, .. }
            | Self::Ellipse { style, .. }
            | Self::Text { style, .. }
            | Self::Marker { style, .. }
            | Self::Highlight { style, .. }
            | Self::HighlightPen { style, .. } => style.color = color,
            Self::Blur { .. } => {}
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Self::Brush { points, .. } | Self::HighlightPen { points, .. } => {
                for p in points {
                    shift(p, dx, dy);
                }
            }
            Self::Line { span, .. }
            | Self::Arrow { span, .. }
            | Self::Rect { span, .. }
            | Self::Ellipse { span, .. }
            | Self::Highlight { span, .. }
            | Self::Blur { span } => span.translate(dx, dy),
            Self::Text { pos, .. } | Self::Marker { pos, .. } => shift(pos, dx, dy),
        }
    }

    // left, top, right, bottom, drawn width included
    pub fn extent(&self, face: &Face) -> [f32; 4] {
        let around = |points: &[Point], pad: f32| {
            let xs = points.iter().map(|p| p.x);
            let ys = points.iter().map(|p| p.y);
            [
                xs.clone().fold(f32::MAX, f32::min) - pad,
                ys.clone().fold(f32::MAX, f32::min) - pad,
                xs.fold(f32::MIN, f32::max) + pad,
                ys.fold(f32::MIN, f32::max) + pad,
            ]
        };
        match self {
            Self::Brush { points, style } => around(points, style.width / 2.0),
            Self::HighlightPen { points, style } => {
                around(points, style.width * HIGHLIGHT_PEN_FACTOR / 2.0)
            }
            Self::Line { span, style } => around(&[span.a, span.b], style.width / 2.0),
            Self::Rect { span, style } | Self::Ellipse { span, style } => {
                let pad = if style.fill { 0.0 } else { style.width / 2.0 };
                around(&[span.a, span.b], pad)
            }
            Self::Arrow { span, style } => {
                let head = ARROW_HEAD_PER_WIDTH * style.width;
                around(&[span.a, span.b], head.max(style.width / 2.0))
            }
            Self::Highlight { span, .. } | Self::Blur { span } => around(&[span.a, span.b], 0.0),
            Self::Marker { pos, style, .. } => {
                around(&[*pos], MARKER_RADIUS_PER_WIDTH * style.width)
            }
            Self::Text { pos, buf, style } => {
                let px = text_px(*style);
                let text = buf.shown();
                let lines = text.split('\n');
                let width = lines
                    .clone()
                    .map(|l| text_width(face, l, px))
                    .fold(0.0, f32::max);
                let height = face.line_height(px) * lines.count() as f32;
                [pos.x, pos.y, pos.x + width, pos.y + height]
            }
        }
    }

    // `extent` is only consulted for text, whose size needs the font
    pub fn hit(&self, p: Point, tol: f32, extent: impl Fn(&Self) -> [f32; 4]) -> bool {
        match self {
            Self::Brush { points, style } => near_polyline(points, p, tol + style.width / 2.0),
            Self::HighlightPen { points, style } => {
                near_polyline(points, p, tol + style.width * HIGHLIGHT_PEN_FACTOR / 2.0)
            }
            Self::Line { span, style } => {
                near_polyline(&[span.a, span.b], p, tol + style.width / 2.0)
            }
            Self::Arrow { span, style } => {
                near_polyline(&[span.a, span.b], p, tol + style.width / 2.0)
                    || dist(p, span.b) <= ARROW_HEAD_PER_WIDTH * style.width
            }
            Self::Rect { span, style } => match span.rect() {
                Some(r) if style.fill => inside(r, p, tol),
                Some(r) => box_distance(r, p).abs() <= tol + style.width / 2.0,
                None => false,
            },
            Self::Ellipse { span, style } => match span.rect() {
                Some(r) => {
                    let reach = ellipse_distance(r, p);
                    if style.fill {
                        reach <= tol
                    } else {
                        reach.abs() <= tol + style.width / 2.0
                    }
                }
                None => false,
            },
            Self::Highlight { span, .. } | Self::Blur { span } => {
                span.rect().is_some_and(|r| inside(r, p, tol))
            }
            Self::Marker { pos, style, .. } => {
                dist(p, *pos) <= MARKER_RADIUS_PER_WIDTH * style.width + tol
            }
            Self::Text { .. } => {
                let [left, top, right, bottom] = extent(self);
                (left - tol..=right + tol).contains(&p.x)
                    && (top - tol..=bottom + tol).contains(&p.y)
            }
        }
    }

    pub fn draw(&self, pm: &mut Pixmap, face: &Face) {
        match self {
            Self::Brush { points, style } => match points.as_slice() {
                [dot] => {
                    let paint = style.color.paint();
                    if let Some(path) = PathBuilder::from_circle(dot.x, dot.y, style.width / 2.0) {
                        pm.fill_path(
                            &path,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                _ => {
                    if let Some(path) = polyline(points) {
                        stroke(pm, &path, *style, LineCap::Round);
                    }
                }
            },
            Self::HighlightPen { points, style } => {
                let wide = Style {
                    color: style.color.with_alpha(HIGHLIGHT_ALPHA),
                    width: style.width * HIGHLIGHT_PEN_FACTOR,
                    fill: false,
                    radius: 0.0,
                    round_caps: false,
                };
                if let Some(path) = polyline(points) {
                    stroke(pm, &path, wide, LineCap::Square);
                }
            }
            Self::Line { span, style } => {
                if let Some(path) = polyline(&[span.a, span.b]) {
                    stroke(pm, &path, *style, style.line_cap());
                }
            }
            Self::Arrow { span, style } => draw_arrow(pm, span.a, span.b, *style),
            Self::Rect { span, style } => {
                let path = span.rect().and_then(|r| rounded_rect(r, style.radius));
                if let Some(path) = path {
                    paint_path(pm, &path, *style, LineCap::Butt);
                }
            }
            Self::Ellipse { span, style } => {
                let path = span.rect().and_then(PathBuilder::from_oval);
                if let Some(path) = path {
                    paint_path(pm, &path, *style, LineCap::Butt);
                }
            }
            Self::Highlight { span, style } => {
                if let Some(rect) = span.rect() {
                    let paint = style.color.with_alpha(HIGHLIGHT_ALPHA).paint();
                    pm.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }
            Self::Blur { span } => {
                if let Some(region) = region(span.a, span.b, pm.width(), pm.height()) {
                    blur(pm, region);
                }
            }
            Self::Text { pos, buf, style } => {
                let px = text_px(*style);
                let step = face.line_height(px);
                let shown = buf.shown();
                let lines: Vec<&str> = shown.split('\n').collect();
                for (i, line) in lines.iter().enumerate() {
                    let top = Point::from_xy(pos.x, pos.y + step * i as f32);
                    draw_line(pm, face, line, top, px, style.color);
                }
                let span = buf.preedit_span();
                if let Some(((i, from, to), line)) =
                    span.and_then(|sp| Some((sp, lines.get(sp.0)?)))
                {
                    let top = Point::from_xy(pos.x, pos.y + step * i as f32);
                    underline(pm, face, top, line, (from, to), *style);
                }
            }
            Self::Marker { pos, number, style } => {
                let radius = MARKER_RADIUS_PER_WIDTH * style.width;
                if let Some(path) = PathBuilder::from_circle(pos.x, pos.y, radius) {
                    let paint = style.color.paint();
                    pm.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                let px = radius * MARKER_TEXT_RATIO;
                draw_centered(pm, face, &number.to_string(), *pos, px, MARKER_TEXT_COLOR);
            }
        }
    }
}

// underlines columns `from..to` of a line, the way input methods mark a preedit
fn underline(
    pm: &mut Pixmap,
    face: &Face,
    top_left: Point,
    line: &str,
    (from, to): (usize, usize),
    style: Style,
) {
    let px = text_px(style);
    let reach = |cols: usize| -> f32 { line.chars().take(cols).map(|c| face.advance(c, px)).sum() };
    let y = top_left.y + face.ascent(px) + UNDERLINE_OFFSET;
    let mut pb = PathBuilder::new();
    pb.move_to(top_left.x + reach(from), y);
    pb.line_to(top_left.x + reach(to), y);
    if let Some(path) = pb.finish() {
        let line_style = Style {
            width: PREEDIT_UNDERLINE_WIDTH,
            fill: false,
            ..style
        };
        stroke(pm, &path, line_style, LineCap::Butt);
    }
}

pub fn compose(base: &Pixmap, shapes: &[Shape], face: &Face) -> Pixmap {
    let mut pm = base.clone();
    for shape in shapes {
        shape.draw(&mut pm, face);
    }
    pm
}

pub fn text_px(style: Style) -> f32 {
    style.width * TEXT_PX_PER_WIDTH
}

pub fn text_width(face: &Face, text: &str, px: f32) -> f32 {
    text.chars().map(|c| face.advance(c, px)).sum()
}

pub fn caret(face: &Face, pos: Point, buf: &TextBuf, style: Style) -> (Point, Point) {
    let px = text_px(style);
    let (line, col) = buf.caret_pos();
    let text = buf.shown();
    let before = text
        .split('\n')
        .nth(line)
        .unwrap_or_default()
        .chars()
        .take(col);
    let x = pos.x + before.map(|c| face.advance(c, px)).sum::<f32>();
    let step = face.line_height(px);
    let top = pos.y + step * line as f32;
    (Point::from_xy(x, top), Point::from_xy(x, top + step))
}

pub fn region(a: Point, b: Point, width: u32, height: u32) -> Option<IntRect> {
    let clamp = |v: f32, max: u32| v.clamp(0.0, max as f32);
    let (x0, x1) = (
        clamp(a.x.min(b.x).round(), width),
        clamp(a.x.max(b.x).round(), width),
    );
    let (y0, y1) = (
        clamp(a.y.min(b.y).round(), height),
        clamp(a.y.max(b.y).round(), height),
    );
    IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)
}

// mean of the new point and the last `size - 1` stored ones
fn smoothed(history: &[Point], at: Point, size: usize) -> Point {
    let keep = size.saturating_sub(1).min(history.len());
    if keep == 0 {
        return at;
    }
    let recent = &history[history.len() - keep..];
    let count = (keep + 1) as f32;
    let sum = recent
        .iter()
        .fold(at, |acc, p| Point::from_xy(acc.x + p.x, acc.y + p.y));
    Point::from_xy(sum.x / count, sum.y / count)
}

fn snap_angle(origin: Point, to: Point) -> Point {
    let (dx, dy) = (to.x - origin.x, to.y - origin.y);
    let step = SNAP_STEP_DEG.to_radians();
    let angle = (dy.atan2(dx) / step).round() * step;
    let len = dx.hypot(dy);
    Point::from_xy(origin.x + len * angle.cos(), origin.y + len * angle.sin())
}

// shift: square or circle, alt: centred on the origin
fn box_corners(origin: Point, to: Point, mods: Mods) -> (Point, Point) {
    let (mut dx, mut dy) = (to.x - origin.x, to.y - origin.y);
    if mods.shift {
        let side = dx.abs().max(dy.abs());
        dx = side.copysign(dx);
        dy = side.copysign(dy);
    }
    let a = if mods.alt {
        Point::from_xy(origin.x - dx, origin.y - dy)
    } else {
        origin
    };
    (a, Point::from_xy(origin.x + dx, origin.y + dy))
}

fn dist(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    dist(p, Point::from_xy(a.x + t * dx, a.y + t * dy))
}

fn near_polyline(points: &[Point], p: Point, reach: f32) -> bool {
    match points {
        [only] => dist(p, *only) <= reach,
        _ => points
            .windows(2)
            .any(|w| segment_distance(p, w[0], w[1]) <= reach),
    }
}

fn inside(r: Rect, p: Point, pad: f32) -> bool {
    (r.left() - pad..=r.right() + pad).contains(&p.x)
        && (r.top() - pad..=r.bottom() + pad).contains(&p.y)
}

// signed distance to the box outline, negative inside
fn box_distance(r: Rect, p: Point) -> f32 {
    let qx = (p.x - f32::midpoint(r.left(), r.right())).abs() - r.width() / 2.0;
    let qy = (p.y - f32::midpoint(r.top(), r.bottom())).abs() - r.height() / 2.0;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0)
}

// rough signed distance to the ellipse outline, negative inside
fn ellipse_distance(r: Rect, p: Point) -> f32 {
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return box_distance(r, p);
    }
    let dx = (p.x - f32::midpoint(r.left(), r.right())) / rx;
    let dy = (p.y - f32::midpoint(r.top(), r.bottom())) / ry;
    (dx.hypot(dy) - 1.0) * rx.min(ry)
}

fn shift(p: &mut Point, dx: f32, dy: f32) {
    p.x += dx;
    p.y += dy;
}

fn polyline(points: &[Point]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.x, first.y);
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    pb.finish()
}

pub fn rounded_rect(rect: Rect, radius: f32) -> Option<Path> {
    let k = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(left + k, top);
    pb.line_to(right - k, top);
    pb.quad_to(right, top, right, top + k);
    pb.line_to(right, bottom - k);
    pb.quad_to(right, bottom, right - k, bottom);
    pb.line_to(left + k, bottom);
    pb.quad_to(left, bottom, left, bottom - k);
    pb.line_to(left, top + k);
    pb.quad_to(left, top, left + k, top);
    pb.close();
    pb.finish()
}

pub fn stroke(pm: &mut Pixmap, path: &Path, style: Style, cap: LineCap) {
    let stroke = Stroke {
        width: style.width,
        line_cap: cap,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pm.stroke_path(
        path,
        &style.color.paint(),
        &stroke,
        Transform::identity(),
        None,
    );
}

fn paint_path(pm: &mut Pixmap, path: &Path, style: Style, cap: LineCap) {
    if style.fill {
        let paint = style.color.paint();
        pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    } else {
        stroke(pm, path, style, cap);
    }
}

fn draw_arrow(pm: &mut Pixmap, from: Point, to: Point, style: Style) {
    let angle = (to.y - from.y).atan2(to.x - from.x);
    let len = ARROW_HEAD_PER_WIDTH * style.width;
    // the shaft stops at the base of the head so the tip stays sharp
    let depth = len * ARROW_HEAD_ANGLE.cos();
    if dist(from, to) > depth {
        let base = Point::from_xy(to.x - depth * angle.cos(), to.y - depth * angle.sin());
        if let Some(path) = polyline(&[from, base]) {
            stroke(pm, &path, style, style.line_cap());
        }
    }
    let wing = |side: f32| {
        let a = angle + side * ARROW_HEAD_ANGLE;
        Point::from_xy(to.x - len * a.cos(), to.y - len * a.sin())
    };
    let mut head = PathBuilder::new();
    head.move_to(to.x, to.y);
    for p in [wing(-1.0), wing(1.0)] {
        head.line_to(p.x, p.y);
    }
    head.close();
    if let Some(path) = head.finish() {
        let paint = style.color.paint();
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

pub fn draw_line(pm: &mut Pixmap, face: &Face, text: &str, top_left: Point, px: f32, color: Color) {
    let baseline = top_left.y + face.ascent(px);
    let mut x = top_left.x;
    for ch in text.chars() {
        let Some(glyph) = face.glyph(ch, px) else {
            continue;
        };
        let (metrics, coverage) = (&glyph.0, &glyph.1);
        if metrics.width > 0 {
            let gx = (x + metrics.xmin as f32).round() as i32;
            let gy = (baseline - metrics.height as f32 - metrics.ymin as f32).round() as i32;
            blend_glyph(pm, (gx, gy), metrics.width, coverage, color);
        }
        x += metrics.advance_width;
    }
}

pub fn draw_centered(
    pm: &mut Pixmap,
    face: &Face,
    text: &str,
    center: Point,
    px: f32,
    color: Color,
) {
    let top_left = Point::from_xy(
        center.x - text_width(face, text, px) / 2.0,
        center.y - face.line_height(px) / 2.0,
    );
    draw_line(pm, face, text, top_left, px, color);
}

// the base is opaque, so blending on premultiplied RGBA is direct
fn blend_glyph(pm: &mut Pixmap, (gx, gy): (i32, i32), width: usize, coverage: &[u8], color: Color) {
    let (pw, ph) = (pm.width() as usize, pm.height() as usize);
    let data = pm.data_mut();
    let max = u32::from(OPAQUE);
    for (i, &cov) in coverage.iter().enumerate() {
        let (x, y) = (gx + (i % width) as i32, gy + (i / width) as i32);
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            continue;
        };
        if x >= pw || y >= ph {
            continue;
        }
        let alpha = u32::from(cov) * u32::from(color.a) / max;
        let at = (y * pw + x) * BYTES_PER_PIXEL;
        let channels = data[at..at + RGB_CHANNELS].iter_mut();
        for (dst, src) in channels.zip([color.r, color.g, color.b]) {
            *dst = ((u32::from(*dst) * (max - alpha) + u32::from(src) * alpha) / max) as u8;
        }
    }
}

fn blur(pm: &mut Pixmap, region: IntRect) {
    let (x, y) = (region.x() as usize, region.y() as usize);
    let (w, h) = (region.width() as usize, region.height() as usize);
    let stride = pm.width() as usize * BYTES_PER_PIXEL;
    let row = w * BYTES_PER_PIXEL;
    let data = pm.data_mut();

    let mut src = vec![0; row * h];
    for (n, line) in src.chunks_exact_mut(row).enumerate() {
        let at = (y + n) * stride + x * BYTES_PER_PIXEL;
        line.copy_from_slice(&data[at..at + row]);
    }
    let mut tmp = vec![0; src.len()];
    box_pass(&src, &mut tmp, (w, h), (BYTES_PER_PIXEL, row));
    box_pass(&tmp, &mut src, (h, w), (row, BYTES_PER_PIXEL));
    for (n, line) in src.chunks_exact(row).enumerate() {
        let at = (y + n) * stride + x * BYTES_PER_PIXEL;
        data[at..at + row].copy_from_slice(line);
    }
}

// steps are in bytes, prefix sums keep it O(n)
fn box_pass(
    src: &[u8],
    dst: &mut [u8],
    (len, lines): (usize, usize),
    (step, line_step): (usize, usize),
) {
    let mut sums = vec![[0u32; BYTES_PER_PIXEL]; len + 1];
    for line in 0..lines {
        let at = |i: usize| line * line_step + i * step;
        for i in 0..len {
            for c in 0..BYTES_PER_PIXEL {
                sums[i + 1][c] = sums[i][c] + u32::from(src[at(i) + c]);
            }
        }
        for i in 0..len {
            let (lo, hi) = (
                i.saturating_sub(BLUR_RADIUS),
                (i + BLUR_RADIUS + 1).min(len),
            );
            let count = (hi - lo) as u32;
            for c in 0..BYTES_PER_PIXEL {
                dst[at(i) + c] = ((sums[hi][c] - sums[lo][c]) / count) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::Fonts;

    const TOL: f32 = 8.0;

    fn pt(x: f32, y: f32) -> Point {
        Point::from_xy(x, y)
    }

    fn checker(size: u32) -> Pixmap {
        let mut pm = Pixmap::new(size, size).unwrap();
        for (i, px) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = if i % 2 == 0 { 0 } else { 255 };
            *px = [v, v, v, OPAQUE];
        }
        pm
    }

    #[test]
    fn blur_stays_in_region() {
        let mut pm = checker(30);
        let before = pm.clone();
        blur(&mut pm, IntRect::from_xywh(5, 5, 10, 10).unwrap());
        for y in 0..30 {
            for x in 0..30 {
                let inside = (5..15).contains(&x) && (5..15).contains(&y);
                let same = pm.pixel(x, y) == before.pixel(x, y);
                assert!(inside || same, "pixel outside the region changed: {x},{y}");
            }
        }
        assert_ne!(pm.data(), before.data());
    }

    #[test]
    fn blur_whole_image() {
        let mut pm = checker(20);
        blur(&mut pm, IntRect::from_xywh(0, 0, 20, 20).unwrap());
    }

    #[test]
    fn region_clamps() {
        let r = region(pt(50.0, 40.0), pt(-5.0, 10.0), 40, 30).unwrap();
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (0, 10, 40, 20));
    }

    #[test]
    fn snap_15() {
        let flat = snap_angle(pt(0.0, 0.0), pt(10.0, 1.0));
        assert!(flat.y.abs() < 1e-4 && flat.x > 9.9);
        let diag = snap_angle(pt(0.0, 0.0), pt(10.0, 9.0));
        assert!((diag.x - diag.y).abs() < 1e-3);
        let len = diag.x.hypot(diag.y);
        assert!((len - 10.0_f32.hypot(9.0)).abs() < 1e-3);
    }

    #[test]
    fn box_modifiers() {
        let mods = Mods {
            shift: true,
            ..Mods::default()
        };
        let (a, b) = box_corners(pt(10.0, 10.0), pt(30.0, 15.0), mods);
        assert_eq!(((a.x, a.y), (b.x, b.y)), ((10.0, 10.0), (30.0, 30.0)));

        let mods = Mods {
            alt: true,
            ..Mods::default()
        };
        let (a, b) = box_corners(pt(10.0, 10.0), pt(30.0, 15.0), mods);
        assert_eq!(((a.x, a.y), (b.x, b.y)), ((-10.0, 5.0), (30.0, 15.0)));

        let mods = Mods {
            shift: true,
            alt: true,
            ctrl: false,
        };
        let (a, b) = box_corners(pt(10.0, 10.0), pt(0.0, 15.0), mods);
        assert_eq!(((a.x, a.y), (b.x, b.y)), ((20.0, 0.0), (0.0, 20.0)));
    }

    fn style(fill: bool) -> Style {
        Style {
            color: crate::config::PALETTE[0],
            width: 4.0,
            fill,
            radius: 0.0,
            round_caps: true,
        }
    }

    fn extra() -> Extra {
        Extra {
            marker: 1,
            highlighter: Highlighter::Block,
        }
    }

    fn shape(tool: Tool, to: Point, fill: bool) -> Shape {
        let mut shape =
            Shape::start(tool, pt(0.0, 0.0), style(fill), Mods::default(), extra()).unwrap();
        shape.drag_to(to, Mods::default(), 0);
        shape
    }

    fn no_extent(_: &Shape) -> [f32; 4] {
        [0.0; 4]
    }

    #[test]
    fn line_hits_along_the_segment_only() {
        let line = shape(Tool::Line, pt(100.0, 100.0), false);
        assert!(line.hit(pt(50.0, 52.0), TOL, no_extent));
        assert!(
            !line.hit(pt(100.0, 0.0), TOL, no_extent),
            "bounding box corner is not the line"
        );
    }

    #[test]
    fn unfilled_rect_hits_the_outline_filled_hits_inside() {
        let hollow = shape(Tool::Rect, pt(100.0, 100.0), false);
        assert!(hollow.hit(pt(50.0, 1.0), TOL, no_extent));
        assert!(!hollow.hit(pt(50.0, 50.0), TOL, no_extent));
        let solid = shape(Tool::Rect, pt(100.0, 100.0), true);
        assert!(solid.hit(pt(50.0, 50.0), TOL, no_extent));
    }

    #[test]
    fn ellipse_outline_and_brush_dot() {
        let ellipse = shape(Tool::Ellipse, pt(100.0, 100.0), false);
        assert!(ellipse.hit(pt(50.0, 0.0), TOL, no_extent));
        assert!(!ellipse.hit(pt(50.0, 50.0), TOL, no_extent));
        let dot = Shape::start(
            Tool::Brush,
            pt(10.0, 10.0),
            style(false),
            Mods::default(),
            extra(),
        )
        .unwrap();
        assert!(!dot.is_degenerate() && dot.hit(pt(12.0, 10.0), TOL, no_extent));
    }

    #[test]
    fn flat_ellipse_is_still_selectable() {
        let flat = shape(Tool::Ellipse, pt(100.0, 0.0), false);
        assert!(flat.hit(pt(50.0, 2.0), TOL, no_extent));
        assert!(!flat.hit(pt(50.0, 60.0), TOL, no_extent));
    }

    #[test]
    fn brush_dot_paints_a_pixel() {
        let Ok(fonts) = Fonts::load(None, None, &[]) else {
            return;
        };
        let face = fonts.text;
        let mut pm = Pixmap::new(20, 20).unwrap();
        let dot = Shape::Brush {
            points: vec![pt(10.0, 10.0)],
            style: style(false),
        };
        dot.draw(&mut pm, &face);
        assert!(pm.pixel(10, 10).unwrap().alpha() > 0);
        assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
    }

    #[test]
    fn rounded_rect_radius_extremes_paint() {
        for radius in [0.0, 1000.0] {
            let rect = Rect::from_ltrb(5.0, 5.0, 35.0, 25.0).unwrap();
            let path = rounded_rect(rect, radius).unwrap();
            let mut pm = Pixmap::new(40, 30).unwrap();
            paint_path(&mut pm, &path, style(true), LineCap::Butt);
            assert!(pm.pixel(20, 15).unwrap().alpha() > 0, "radius {radius}");
        }
    }

    #[test]
    fn glyph_clipped_at_the_edges() {
        let mut pm = Pixmap::new(10, 10).unwrap();
        pm.fill(tiny_skia::Color::BLACK);
        let coverage = [OPAQUE; 36];
        blend_glyph(&mut pm, (-3, 8), 6, &coverage, crate::config::PALETTE[0]);
        blend_glyph(&mut pm, (8, -3), 6, &coverage, crate::config::PALETTE[0]);
        assert!(pm.pixel(0, 9).unwrap().red() > 0);
    }

    #[test]
    fn smoothing_averages_recent_points() {
        let history = [pt(0.0, 0.0), pt(10.0, 0.0)];
        let raw = pt(20.0, 0.0);
        assert_eq!(smoothed(&history, raw, 0).x, 20.0);
        assert_eq!(smoothed(&history, raw, 1).x, 20.0);
        assert_eq!(smoothed(&history, raw, 2).x, 15.0);
        assert_eq!(smoothed(&history, raw, 3).x, 10.0);
        assert_eq!(smoothed(&history, raw, 50).x, 10.0);
        assert_eq!(smoothed(&[], raw, 5).x, 20.0);
    }

    #[test]
    fn brush_drag_uses_smoothing() {
        let mut brush = Shape::start(
            Tool::Brush,
            pt(0.0, 0.0),
            style(false),
            Mods::default(),
            extra(),
        )
        .unwrap();
        brush.drag_to(pt(10.0, 0.0), Mods::default(), 2);
        let Shape::Brush { points, .. } = &brush else {
            panic!("not a brush")
        };
        assert_eq!(points[1].x, 5.0);
    }
}
