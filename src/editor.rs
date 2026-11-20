use anyhow::{Context, Result};
use smithay_client_toolkit::seat::keyboard::Keysym;
use tiny_skia::{
    FillRule, FilterQuality, IntRect, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Point,
    Rect, Transform,
};

use crate::{
    config::{
        BORDER_COLOR, BORDER_WIDTH, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, CARET_WIDTH, COLOR_KEYS,
        DEFAULT_SIZE, DIM_ALPHA, FACTOR_COARSE_STEP, FACTOR_FINE_STEP, FACTOR_MAX, FACTOR_MIN,
        FACTOR_STEP, HANDLE_HIT, HANDLE_SIZE, HIT_TOLERANCE, LATIN_KEYS, SCROLL_PAN, SELECT_COLOR,
        SELECT_PAD, SELECT_WIDTH, SIZES, VIEW_BACKGROUND,
    },
    font::Fonts,
    history::History,
    output,
    settings::{Act, Highlighter, Settings},
    style::{Color, Style},
    text::TextBuf,
    tools::{self, Extra, Mods, Shape, TOOL_KEYS, Tool},
    ui::{Active, Item, Toolbars},
    view::View,
};

pub enum Request {
    Run(Vec<Act>),
    PickColor,
}

#[derive(Clone, Copy)]
enum Phase {
    Select { drag_from: Option<Point> },
    Edit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pull {
    Low,
    Fixed,
    High,
}

// what a crop drag moves: one edge or corner, or the whole selection
#[derive(Clone, Copy)]
enum Handle {
    Move,
    Resize(Pull, Pull),
}

#[derive(Clone, Copy)]
enum Grab {
    None,
    Draw,
    Move {
        index: usize,
        last: Point,
        total: (f32, f32),
    },
    Crop {
        handle: Handle,
        start: IntRect,
        origin: Point,
    },
    Pan {
        last: Point,
    },
}

pub struct Editor {
    base: Pixmap,
    fonts: Fonts,
    screen: (u32, u32),
    view: View,
    scene: Pixmap,
    scene_version: Option<u64>,
    work: Pixmap,
    frame: Pixmap,

    phase: Phase,
    selection: Option<IntRect>,
    crop_backup: Option<IntRect>,
    tool: Tool,
    color: Color,
    fill: bool,
    size: usize,
    factor: f32,
    radius: f32,
    round_caps: bool,
    highlighter: Highlighter,
    palette: Vec<Color>,
    on_enter: Vec<Act>,
    on_escape: Vec<Act>,
    on_right_click: Vec<Act>,
    tool_keys: [(char, Tool); TOOL_KEYS.len()],
    smoothing: usize,
    zoom_factor: f32,
    pan_step: f32,
    text_move: f32,
    seen_version: u64,

    history: History,
    draft: Option<Shape>,
    grab: Grab,
    picked: Option<usize>,
    mods: Mods,
    toolbars: bool,
}

impl Editor {
    // `whole` starts with the full image selected, for annotating an existing file
    pub fn new(base: Pixmap, fonts: Fonts, settings: &Settings, whole: bool) -> Result<Self> {
        let size = (base.width(), base.height());
        let selection = whole
            .then(|| IntRect::from_xywh(0, 0, size.0, size.1))
            .flatten();
        let blank = || Pixmap::new(size.0, size.1).context("empty image");
        Ok(Self {
            scene: base.clone(),
            work: blank()?,
            frame: blank()?,
            base,
            fonts,
            screen: size,
            view: View::new(size, size),
            scene_version: None,
            phase: if whole {
                Phase::Edit
            } else {
                Phase::Select { drag_from: None }
            },
            selection,
            crop_backup: selection.filter(|_| settings.initial_tool == Tool::Crop),
            tool: settings.initial_tool,
            color: settings.palette[0],
            fill: settings.fill_shapes,
            size: DEFAULT_SIZE,
            factor: settings.size_factor,
            radius: settings.corner_roundness,
            round_caps: settings.round_caps,
            highlighter: settings.highlighter,
            palette: settings.palette.clone(),
            on_enter: settings.on_enter.clone(),
            on_escape: settings.on_escape.clone(),
            on_right_click: settings.on_right_click.clone(),
            tool_keys: settings.tool_keys,
            smoothing: settings.smoothing,
            zoom_factor: settings.zoom_factor,
            pan_step: settings.pan_step,
            text_move: settings.text_move,
            seen_version: 0,
            history: History::default(),
            draft: None,
            grab: Grab::None,
            picked: None,
            mods: Mods::default(),
            toolbars: !settings.hide_toolbars,
        })
    }

    pub fn set_screen(&mut self, screen: (u32, u32)) -> Result<()> {
        self.screen = screen;
        self.view = View::new((self.base.width(), self.base.height()), screen);
        self.frame = Pixmap::new(screen.0, screen.1).context("empty screen")?;
        Ok(())
    }

    // a text draft is open, so an input method should be active
    pub fn typing(&self) -> bool {
        self.draft.as_ref().is_some_and(Shape::is_text)
    }

    pub fn ime_update(&mut self, delete: (usize, usize), commit: &str, preedit: &str) {
        if let Some(buf) = self.draft.as_mut().and_then(Shape::text_mut) {
            buf.apply_ime(delete, commit, preedit);
        }
    }

    // draft text and cursor byte offset, as the input method wants them
    pub fn ime_surrounding(&self) -> Option<(String, usize)> {
        self.draft
            .as_ref()
            .and_then(Shape::text)
            .map(TextBuf::surrounding)
    }

    // x, y, width, height of the caret on screen, where the input method places its popup
    pub fn ime_cursor(&self) -> Option<[f32; 4]> {
        let Some(Shape::Text { pos, buf, style }) = &self.draft else {
            return None;
        };
        let (top, bottom) = tools::caret(&self.fonts.text, *pos, buf, *style);
        let (top, bottom) = (self.view.to_screen(top), self.view.to_screen(bottom));
        Some([top.x, top.y, CARET_WIDTH, bottom.y - top.y])
    }

    #[cfg(test)]
    pub fn color(&self) -> Color {
        self.color
    }

    pub fn toolbars_shown(&self) -> bool {
        self.toolbars
    }

    pub fn set_toolbars(&mut self, shown: bool) {
        self.toolbars = shown;
    }

    // true when annotations changed since the last call; drives --auto-copy
    pub fn annotations_changed(&mut self) -> bool {
        let version = self.history.version();
        std::mem::replace(&mut self.seen_version, version) != version
    }

    pub fn set_mods(&mut self, mods: Mods) {
        self.mods = mods;
    }

    pub fn set_color(&mut self, color: Color) {
        self.color = color;
        if let Some(shape) = self.draft.as_mut().filter(|s| s.is_text()) {
            shape.set_color(color);
        }
    }

    pub fn export(&mut self) -> Result<Vec<u8>> {
        self.commit_draft();
        let image = tools::compose(&self.base, self.history.shapes(), &self.fonts.text);
        let region = match self.selection {
            Some(r) => r,
            None => IntRect::from_xywh(0, 0, self.base.width(), self.base.height())
                .context("empty image")?,
        };
        output::crop_png(&image, region)
    }

    pub fn render(&mut self) -> &Pixmap {
        let face = &self.fonts.text;
        if self.scene_version != Some(self.history.version()) {
            self.scene = tools::compose(&self.base, self.history.shapes(), face);
            self.scene_version = Some(self.history.version());
        }
        self.work.data_mut().copy_from_slice(self.scene.data());
        if let Some(draft) = &self.draft {
            draft.draw(&mut self.work, face);
            if let Shape::Text { pos, buf, style } = draft {
                let (top, bottom) = tools::caret(face, *pos, buf, *style);
                let mut pb = PathBuilder::new();
                pb.move_to(top.x, top.y);
                pb.line_to(bottom.x, bottom.y);
                if let Some(path) = pb.finish() {
                    let caret = Style {
                        width: CARET_WIDTH,
                        fill: false,
                        ..*style
                    };
                    tools::stroke(&mut self.work, &path, caret, LineCap::Butt);
                }
            }
        }
        // chrome keeps its size on screen whatever the zoom
        let unit = 1.0 / self.view.zoom();
        draw_dim(&mut self.work, self.selection, unit);
        if let Some(selection) = self.selection.filter(|_| self.tool == Tool::Crop) {
            draw_handles(&mut self.work, selection, unit);
        }
        if let Some(shape) = self.picked.and_then(|i| self.history.shapes().get(i)) {
            draw_outline(&mut self.work, shape.extent(face), unit);
        }

        let bars = self.bars();
        let active = Active {
            palette: &self.palette,
            tool: self.tool,
            color: self.color,
            fill: self.fill,
            size: self.size,
        };
        if self.view.is_identity() {
            if let Some(bars) = bars {
                bars.draw(&mut self.work, &self.fonts, &active);
            }
            return &self.work;
        }
        self.frame.fill(VIEW_BACKGROUND.to_skia());
        let zoom = self.view.zoom();
        let (pan_x, pan_y) = self.view.pan();
        let quality = if (zoom - 1.0).abs() < f32::EPSILON {
            FilterQuality::Nearest
        } else {
            FilterQuality::Bilinear
        };
        let paint = PixmapPaint {
            quality,
            ..PixmapPaint::default()
        };
        let transform = Transform::from_row(zoom, 0.0, 0.0, zoom, pan_x, pan_y);
        self.frame
            .draw_pixmap(0, 0, self.work.as_ref(), &paint, transform, None);
        if let Some(bars) = bars {
            bars.draw(&mut self.frame, &self.fonts, &active);
        }
        &self.frame
    }

    pub fn key(&mut self, sym: Keysym, raw: u32, text: Option<&str>) -> Option<Request> {
        if self.typing() && self.text_key(sym, raw, text) {
            return None;
        }
        if let Some((dx, dy)) = arrow_direction(sym).filter(|_| self.mods.alt) {
            self.view.pan_by(dx * self.pan_step, dy * self.pan_step);
            return None;
        }
        let moved = arrow_direction(sym).zip(self.picked);
        if let Some(((dx, dy), index)) = moved {
            // the arrows point the other way than the pan direction
            let (dx, dy) = (-dx * self.text_move, -dy * self.text_move);
            self.history.nudge(index, dx, dy);
            self.history.record_move(index, (dx, dy));
            return None;
        }
        match sym {
            Keysym::Up | Keysym::Page_Up => self.step_factor(1.0),
            Keysym::Down | Keysym::Page_Down => self.step_factor(-1.0),
            _ if self.mods.ctrl => return self.ctrl_key(shortcut(sym, raw)),
            Keysym::Escape => return self.escape(),
            Keysym::Return | Keysym::KP_Enter => return self.enter(),
            Keysym::Delete | Keysym::KP_Delete => self.delete_picked(),
            other if !self.mods.alt => self.plain_char(other.key_char()),
            _ => {}
        }
        None
    }

    // only text editing keys repeat, never one-shot actions
    pub fn repeat(&mut self, sym: Keysym, raw: u32, text: Option<&str>) -> bool {
        let one_shot = matches!(sym, Keysym::Return | Keysym::KP_Enter | Keysym::Escape);
        let editing = self.typing();
        editing && !one_shot && !self.mods.ctrl && self.text_key(sym, raw, text)
    }

    pub fn press(&mut self, at: Point, button: u32) -> Option<Request> {
        match button {
            BTN_RIGHT if self.on_right_click.is_empty() => return None,
            BTN_RIGHT => return Some(Request::Run(self.on_right_click.clone())),
            BTN_MIDDLE => {
                self.grab = Grab::Pan { last: at };
                return None;
            }
            BTN_LEFT => {}
            _ => return None,
        }
        if let Some(bars) = self.bars() {
            if let Some(item) = bars.hit(at) {
                return self.apply_item(item);
            }
            if bars.covers(at) {
                return None;
            }
        }
        let p = self.view.to_image(at);
        match self.phase {
            Phase::Select { .. } => {
                self.selection = None;
                self.phase = Phase::Select { drag_from: Some(p) };
            }
            Phase::Edit => self.press_edit(p),
        }
        None
    }

    // true when the picture changed
    pub fn motion(&mut self, at: Point) -> bool {
        if let Grab::Pan { last } = self.grab {
            self.view.pan_by(at.x - last.x, at.y - last.y);
            self.grab = Grab::Pan { last: at };
            return true;
        }
        let p = self.view.to_image(at);
        let (w, h) = (self.base.width(), self.base.height());
        if let Phase::Select {
            drag_from: Some(from),
        } = self.phase
        {
            self.selection = tools::region(from, p, w, h);
            return true;
        }
        match self.grab {
            Grab::Draw => {
                if let Some(shape) = &mut self.draft {
                    shape.drag_to(p, self.mods, self.smoothing);
                }
            }
            Grab::Move { index, last, total } => {
                let (dx, dy) = (p.x - last.x, p.y - last.y);
                self.history.nudge(index, dx, dy);
                self.grab = Grab::Move {
                    index,
                    last: p,
                    total: (total.0 + dx, total.1 + dy),
                };
            }
            Grab::Crop {
                handle,
                start,
                origin,
            } => {
                let delta = (p.x - origin.x, p.y - origin.y);
                self.selection = crop_drag(start, handle, delta, (w, h)).or(self.selection);
            }
            Grab::None | Grab::Pan { .. } => return false,
        }
        true
    }

    pub fn release(&mut self, button: u32) {
        let panning = matches!(self.grab, Grab::Pan { .. });
        if button == BTN_MIDDLE && panning {
            self.grab = Grab::None;
        }
        if button != BTN_LEFT || panning {
            return;
        }
        match std::mem::replace(&mut self.grab, Grab::None) {
            Grab::Draw => self.commit_draft(),
            Grab::Move { index, total, .. } if total != (0.0, 0.0) => {
                self.history.record_move(index, total);
            }
            _ => {}
        }
        if let Phase::Select { drag_from } = &mut self.phase {
            *drag_from = None;
        }
    }

    // wheel over the factor buttons resizes, with Ctrl it zooms, otherwise it pans
    pub fn scroll(&mut self, at: Point, (dx, dy): (f32, f32)) {
        let factor_button = self
            .bars()
            .and_then(|bars| bars.hit(at))
            .is_some_and(|item| matches!(item, Item::FactorDown | Item::FactorUp));
        if factor_button {
            self.step_factor(if dy < 0.0 { 1.0 } else { -1.0 });
        } else if self.mods.ctrl {
            let zoom = if dy < 0.0 {
                self.zoom_factor
            } else {
                1.0 / self.zoom_factor
            };
            self.view.zoom_at(at, zoom);
        } else {
            self.view.pan_by(-dx * SCROLL_PAN, -dy * SCROLL_PAN);
        }
    }

    fn bars(&self) -> Option<Toolbars> {
        let dragging = matches!(self.phase, Phase::Select { drag_from: Some(_) });
        let selection = self.selection.filter(|_| self.toolbars && !dragging)?;
        let corner = |x: i32, y: i32| self.view.to_screen(Point::from_xy(x as f32, y as f32));
        let area = tools::region(
            corner(selection.left(), selection.top()),
            corner(selection.right(), selection.bottom()),
            self.screen.0,
            self.screen.1,
        )?;
        Some(Toolbars::layout(area, self.screen, self.palette.len()))
    }

    fn style(&self) -> Style {
        Style {
            color: self.color,
            width: SIZES[self.size] * self.factor,
            fill: self.fill,
            radius: self.radius,
            round_caps: self.round_caps,
        }
    }

    fn cropping(&self) -> bool {
        self.tool == Tool::Crop && matches!(self.phase, Phase::Edit)
    }

    fn commit_draft(&mut self) {
        if let Some(mut shape) = self.draft.take().filter(|s| !s.is_degenerate()) {
            if let Some(buf) = shape.text_mut() {
                buf.clear_preedit();
            }
            self.history.add(shape);
            self.picked = None;
        }
    }

    // drops an unfinished drag, putting back whatever it had already changed
    fn cancel_grab(&mut self) {
        match std::mem::replace(&mut self.grab, Grab::None) {
            Grab::Draw => self.draft = None,
            Grab::Move { index, total, .. } => {
                self.history.nudge(index, -total.0, -total.1);
            }
            Grab::Crop { start, .. } => self.selection = Some(start),
            Grab::None | Grab::Pan { .. } => {}
        }
    }

    // an unfinished drag or open draft goes first
    fn undo(&mut self) {
        let dragging = !matches!(self.grab, Grab::None);
        self.cancel_grab();
        if !dragging && self.draft.take().is_none() {
            self.history.undo();
        }
        self.picked = None;
    }

    fn redo(&mut self) {
        self.cancel_grab();
        self.history.redo();
        self.picked = None;
    }

    fn set_tool(&mut self, tool: Tool) {
        if self.selection.is_none() {
            return;
        }
        self.commit_draft();
        self.picked = None;
        if tool != Tool::Crop {
            self.crop_backup = None;
        } else if self.tool != Tool::Crop {
            self.crop_backup = self.selection;
        }
        self.phase = Phase::Edit;
        self.tool = tool;
    }

    fn bump_factor(&mut self, delta: f32) {
        self.factor = (self.factor + delta).clamp(FACTOR_MIN, FACTOR_MAX);
    }

    fn step_factor(&mut self, direction: f32) {
        let step = if self.mods.shift {
            FACTOR_FINE_STEP
        } else if self.mods.ctrl {
            FACTOR_COARSE_STEP
        } else {
            FACTOR_STEP
        };
        self.bump_factor(direction * step);
    }

    fn apply_item(&mut self, item: Item) -> Option<Request> {
        match item {
            Item::Tool(tool) => self.set_tool(tool),
            Item::Undo => self.undo(),
            Item::Redo => self.redo(),
            Item::Copy => return run(Act::SaveToClipboard),
            Item::Save => return run(Act::SaveToFile),
            Item::Swatch(i) => self.set_color(self.palette[i]),
            Item::Custom => return Some(Request::PickColor),
            Item::Size(i) => self.size = i,
            Item::FactorDown => self.bump_factor(-FACTOR_STEP),
            Item::FactorUp => self.bump_factor(FACTOR_STEP),
            Item::Fill => self.fill = !self.fill,
        }
        None
    }

    // true when the text editor took the key
    fn text_key(&mut self, sym: Keysym, raw: u32, text: Option<&str>) -> bool {
        let Mods { shift, ctrl, .. } = self.mods;
        if ctrl && matches!(shortcut(sym, raw), Some('z' | 'y')) {
            return true;
        }
        let Some(buf) = self.draft.as_mut().and_then(Shape::text_mut) else {
            return false;
        };
        match sym {
            Keysym::BackSpace if ctrl => buf.word_backspace(),
            _ if ctrl => return false,
            Keysym::Return | Keysym::KP_Enter if shift => buf.newline(),
            Keysym::Return | Keysym::KP_Enter => self.commit_draft(),
            Keysym::Escape => self.draft = None,
            Keysym::Left => buf.left(),
            Keysym::Right => buf.right(),
            Keysym::Up => buf.up(),
            Keysym::Down => buf.down(),
            Keysym::Home => buf.home(),
            Keysym::End => buf.end(),
            Keysym::BackSpace => buf.backspace(),
            Keysym::Delete | Keysym::KP_Delete => buf.delete(),
            _ => {
                if let Some(text) = text {
                    buf.insert(text);
                }
            }
        }
        true
    }

    fn ctrl_key(&mut self, letter: Option<char>) -> Option<Request> {
        let Mods { shift, alt, .. } = self.mods;
        match letter? {
            'c' if alt => run(Act::CopyFilepathToClipboard),
            'c' => run(Act::SaveToClipboard),
            's' if shift => run(Act::SaveToFileAs),
            's' => run(Act::SaveToFile),
            'z' => {
                self.undo();
                None
            }
            'y' => {
                self.redo();
                None
            }
            't' => {
                self.toolbars = !self.toolbars;
                None
            }
            _ => None,
        }
    }

    fn escape(&mut self) -> Option<Request> {
        if self.draft.take().is_some() {
            return None;
        }
        if self.cropping() {
            self.selection = self.crop_backup.take().or(self.selection);
            self.tool = Tool::DEFAULT;
            return None;
        }
        Some(Request::Run(self.on_escape.clone()))
    }

    fn enter(&mut self) -> Option<Request> {
        if self.cropping() {
            self.crop_backup = None;
            self.tool = Tool::DEFAULT;
            return None;
        }
        Some(Request::Run(self.on_enter.clone()))
    }

    fn delete_picked(&mut self) {
        match self.picked.take() {
            Some(index) => self.history.remove(index),
            None => self.history.clear(),
        }
    }

    fn plain_char(&mut self, ch: Option<char>) {
        let Some(ch) = ch else { return };
        let tool = self.tool_keys.iter().find(|(key, _)| *key == ch);
        if let Some(&(_, tool)) = tool {
            self.set_tool(tool);
        } else if let Some(color) = COLOR_KEYS
            .iter()
            .position(|k| *k == ch)
            .and_then(|i| self.palette.get(i))
        {
            self.set_color(*color);
        }
    }

    fn press_edit(&mut self, p: Point) {
        self.commit_draft();
        let unit = 1.0 / self.view.zoom();
        if self.tool == Tool::Crop {
            let grabbed = self
                .selection
                .and_then(|start| Some((start, handle_at(start, p, HANDLE_HIT * unit)?)));
            if let Some((start, handle)) = grabbed {
                self.grab = Grab::Crop {
                    handle,
                    start,
                    origin: p,
                };
            }
            return;
        }
        if !self.selection.is_some_and(|r| inside(r, p)) {
            return;
        }
        if self.tool == Tool::Pointer {
            let face = &self.fonts.text;
            let hit = |shape: &Shape| shape.hit(p, HIT_TOLERANCE * unit, |s| s.extent(face));
            self.picked = self.history.shapes().iter().rposition(hit);
            if let Some(index) = self.picked {
                self.grab = Grab::Move {
                    index,
                    last: p,
                    total: (0.0, 0.0),
                };
            }
            return;
        }
        let extra = Extra {
            marker: self.history.next_marker(),
            highlighter: self.highlighter,
        };
        self.draft = Shape::start(self.tool, p, self.style(), self.mods, extra);
        match &self.draft {
            Some(Shape::Marker { .. }) => self.commit_draft(),
            Some(shape) if !shape.is_text() => self.grab = Grab::Draw,
            _ => {}
        }
    }
}

#[allow(clippy::unnecessary_wraps)] // keeps the call sites as `return run(..)`
fn run(act: Act) -> Option<Request> {
    Some(Request::Run(vec![act]))
}

// Ctrl shortcuts match the Latin letter even on non-Latin layouts
fn shortcut(sym: Keysym, raw: u32) -> Option<char> {
    sym.key_char()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .or_else(|| {
            LATIN_KEYS
                .iter()
                .find(|(code, _)| *code == raw)
                .map(|(_, c)| *c)
        })
}

fn arrow_direction(sym: Keysym) -> Option<(f32, f32)> {
    match sym {
        Keysym::Left => Some((1.0, 0.0)),
        Keysym::Right => Some((-1.0, 0.0)),
        Keysym::Up => Some((0.0, 1.0)),
        Keysym::Down => Some((0.0, -1.0)),
        _ => None,
    }
}

fn inside(r: IntRect, p: Point) -> bool {
    (r.left() as f32..=r.right() as f32).contains(&p.x)
        && (r.top() as f32..=r.bottom() as f32).contains(&p.y)
}

// the 8 anchors (corners and edge midpoints) with the handle each one drags
fn anchors(r: IntRect) -> impl Iterator<Item = (Handle, Point)> {
    const PULLS: [Pull; 3] = [Pull::Low, Pull::Fixed, Pull::High];
    let xs = [
        r.left() as f32,
        (r.left() + r.right()) as f32 / 2.0,
        r.right() as f32,
    ];
    let ys = [
        r.top() as f32,
        (r.top() + r.bottom()) as f32 / 2.0,
        r.bottom() as f32,
    ];
    (0..3)
        .flat_map(|iy| (0..3).map(move |ix| (ix, iy)))
        .filter(|&cell| cell != (1, 1))
        .map(move |(ix, iy)| {
            (
                Handle::Resize(PULLS[ix], PULLS[iy]),
                Point::from_xy(xs[ix], ys[iy]),
            )
        })
}

fn handle_at(r: IntRect, p: Point, reach: f32) -> Option<Handle> {
    let near = |(_, a): &(Handle, Point)| (p.x - a.x).abs() <= reach && (p.y - a.y).abs() <= reach;
    anchors(r)
        .find(near)
        .map(|(h, _)| h)
        .or_else(|| inside(r, p).then_some(Handle::Move))
}

fn crop_drag(
    start: IntRect,
    handle: Handle,
    (dx, dy): (f32, f32),
    (width, height): (u32, u32),
) -> Option<IntRect> {
    let (mut left, mut top) = (start.left() as f32, start.top() as f32);
    let (mut right, mut bottom) = (start.right() as f32, start.bottom() as f32);
    let (dx, dy) = (dx.round(), dy.round());
    match handle {
        Handle::Move => {
            let dx = dx.clamp(-left, width as f32 - right);
            let dy = dy.clamp(-top, height as f32 - bottom);
            (left, right, top, bottom) = (left + dx, right + dx, top + dy, bottom + dy);
        }
        Handle::Resize(horizontal, vertical) => {
            let pull = |how: Pull, low: &mut f32, high: &mut f32, d: f32| match how {
                Pull::Low => *low += d,
                Pull::High => *high += d,
                Pull::Fixed => {}
            };
            pull(horizontal, &mut left, &mut right, dx);
            pull(vertical, &mut top, &mut bottom, dy);
        }
    }
    tools::region(
        Point::from_xy(left, top),
        Point::from_xy(right, bottom),
        width,
        height,
    )
}

fn draw_dim(frame: &mut Pixmap, selection: Option<IntRect>, unit: f32) {
    let full = Rect::from_xywh(0.0, 0.0, frame.width() as f32, frame.height() as f32);
    let sel = selection.map(|r| r.to_rect());
    let mut pb = PathBuilder::new();
    for rect in full.iter().chain(sel.iter()) {
        pb.push_rect(*rect);
    }
    if let Some(path) = pb.finish() {
        let mut dim = Paint::default();
        dim.set_color_rgba8(0, 0, 0, DIM_ALPHA);
        frame.fill_path(&path, &dim, FillRule::EvenOdd, Transform::identity(), None);
    }
    if let Some(rect) = sel {
        let style = Style {
            color: BORDER_COLOR,
            width: BORDER_WIDTH * unit,
            fill: false,
            radius: 0.0,
            round_caps: false,
        };
        tools::stroke(frame, &PathBuilder::from_rect(rect), style, LineCap::Butt);
    }
}

fn draw_handles(frame: &mut Pixmap, selection: IntRect, unit: f32) {
    let paint = BORDER_COLOR.paint();
    let size = HANDLE_SIZE * unit;
    let half = size / 2.0;
    for (_, a) in anchors(selection) {
        if let Some(rect) = Rect::from_xywh(a.x - half, a.y - half, size, size) {
            frame.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }
}

fn draw_outline(frame: &mut Pixmap, [l, t, r, b]: [f32; 4], unit: f32) {
    let pad = SELECT_PAD * unit;
    let rect = Rect::from_ltrb(l - pad, t - pad, r + pad, b + pad);
    if let Some(rect) = rect {
        let style = Style {
            color: SELECT_COLOR,
            width: SELECT_WIDTH * unit,
            fill: false,
            radius: 0.0,
            round_caps: false,
        };
        tools::stroke(frame, &PathBuilder::from_rect(rect), style, LineCap::Butt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: u32, h: u32) -> IntRect {
        IntRect::from_xywh(x, y, w, h).unwrap()
    }

    #[test]
    fn corner_drag_resizes() {
        let sel = rect(100, 100, 200, 100);
        let (corner, _) = anchors(sel).last().unwrap();
        let r = crop_drag(sel, corner, (50.0, 30.0), (1920, 1080)).unwrap();
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (100, 100, 250, 130));
    }

    #[test]
    fn move_keeps_size_and_stays_inside() {
        let r = crop_drag(
            rect(100, 100, 200, 100),
            Handle::Move,
            (-500.0, 5000.0),
            (1920, 1080),
        )
        .unwrap();
        assert_eq!(
            (r.x(), r.width(), r.height(), r.bottom()),
            (0, 200, 100, 1080)
        );
    }

    #[test]
    fn handle_lookup() {
        let sel = rect(100, 100, 200, 100);
        let corner = handle_at(sel, Point::from_xy(101.0, 99.0), HANDLE_HIT);
        assert!(matches!(corner, Some(Handle::Resize(Pull::Low, Pull::Low))));
        let inner = handle_at(sel, Point::from_xy(200.0, 150.0), HANDLE_HIT);
        assert!(matches!(inner, Some(Handle::Move)));
        assert!(handle_at(sel, Point::from_xy(10.0, 10.0), HANDLE_HIT).is_none());
    }

    #[test]
    fn shortcut_falls_back_to_the_physical_key() {
        assert_eq!(shortcut(Keysym::c, 0), Some('c'));
        assert_eq!(shortcut(Keysym::S, 0), Some('s'));
        assert_eq!(shortcut(Keysym::Cyrillic_es, 46), Some('c'));
        assert_eq!(shortcut(Keysym::Cyrillic_es, 1), None);
    }

    // None when fontconfig has no font, so the tests skip
    fn editor(size: u32, whole: bool) -> Option<Editor> {
        let base = Pixmap::new(size, size).unwrap();
        let fonts = Fonts::load(None, None, &[]).ok()?;
        Some(Editor::new(base, fonts, &Settings::for_tests().unwrap(), whole).unwrap())
    }

    #[test]
    fn middle_release_ends_the_pan() {
        let Some(mut editor) = editor(200, false) else {
            return;
        };
        assert!(
            editor
                .press(Point::from_xy(10.0, 10.0), BTN_MIDDLE)
                .is_none()
        );
        assert!(editor.motion(Point::from_xy(20.0, 20.0)));
        editor.release(BTN_MIDDLE);
        assert!(!editor.motion(Point::from_xy(30.0, 30.0)));
    }

    #[test]
    fn other_buttons_do_not_end_a_pan() {
        let Some(mut editor) = editor(200, false) else {
            return;
        };
        editor.press(Point::from_xy(10.0, 10.0), BTN_MIDDLE);
        editor.release(BTN_LEFT);
        assert!(editor.motion(Point::from_xy(20.0, 20.0)));
    }

    #[test]
    fn annotation_changes_are_reported_once() {
        let Some(mut editor) = editor(1200, true) else {
            return;
        };
        assert!(!editor.annotations_changed());
        editor.press(Point::from_xy(600.0, 600.0), BTN_LEFT);
        editor.motion(Point::from_xy(700.0, 700.0));
        editor.release(BTN_LEFT);
        assert!(editor.annotations_changed());
        assert!(!editor.annotations_changed());
    }

    #[test]
    fn arrows_move_the_picked_shape() {
        let Some(mut editor) = editor(1200, true) else {
            return;
        };
        editor.press(Point::from_xy(600.0, 600.0), BTN_LEFT);
        editor.motion(Point::from_xy(700.0, 600.0));
        editor.release(BTN_LEFT);
        editor.set_tool(Tool::Pointer);
        editor.press(Point::from_xy(650.0, 600.0), BTN_LEFT);
        editor.release(BTN_LEFT);
        editor.annotations_changed();
        editor.key(Keysym::Right, 0, None);
        assert!(editor.annotations_changed());
        editor.undo();
        assert!(editor.annotations_changed());
    }

    #[test]
    fn ime_text_lands_in_the_open_draft() {
        let Some(mut editor) = editor(1200, true) else {
            return;
        };
        editor.set_tool(Tool::Text);
        editor.press(Point::from_xy(600.0, 600.0), BTN_LEFT);
        assert!(editor.typing());
        editor.ime_update((0, 0), "é你", "ni");
        assert_eq!(editor.ime_surrounding(), Some(("é你".to_owned(), 5)));
        editor.ime_update((0, 0), "", "");
        assert!(editor.ime_cursor().is_some());
    }
}
