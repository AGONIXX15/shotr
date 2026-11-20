use tiny_skia::{FillRule, IntRect, Pixmap, Point, Rect, Transform};

use crate::{
    config::{
        BAR_COLOR, BAR_MARGIN, BAR_PADDING, BAR_RADIUS, BUTTON_ACTIVE_COLOR, BUTTON_GAP,
        BUTTON_RADIUS, BUTTON_SIZE, BUTTON_TEXT_COLOR, ICON_COPY, ICON_CUSTOM_COLOR, ICON_FILL,
        ICON_PX, ICON_REDO, ICON_SAVE, ICON_UNDO, LABEL_COPY, LABEL_CUSTOM_COLOR,
        LABEL_FACTOR_DOWN, LABEL_FACTOR_UP, LABEL_FILL, LABEL_PX, LABEL_REDO, LABEL_SAVE,
        LABEL_UNDO, SIZE_LABELS, SWATCH_INSET, TOOL_ICONS, TOOL_LABELS,
    },
    font::Fonts,
    style::Color,
    tools::{self, TOOL_KEYS, Tool},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Tool(Tool),
    Undo,
    Redo,
    Copy,
    Save,
    Swatch(usize),
    Custom,
    Size(usize),
    FactorDown,
    FactorUp,
    Fill,
}

#[derive(Clone, Copy)]
pub struct Area {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Area {
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    fn rect(self) -> Option<Rect> {
        Rect::from_xywh(self.x, self.y, self.w, self.h)
    }

    fn center(self) -> Point {
        Point::from_xy(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

pub struct Button {
    area: Area,
    item: Item,
}

pub struct Toolbars {
    bars: [Area; 2],
    buttons: Vec<Button>,
}

// what the bars highlight as active
pub struct Active<'a> {
    pub palette: &'a [Color],
    pub tool: Tool,
    pub color: Color,
    pub fill: bool,
    pub size: usize,
}

const TOP_ITEMS: [Item; 4] = [Item::Undo, Item::Redo, Item::Copy, Item::Save];
const SIZE_ITEMS: [Item; 3] = [Item::Size(0), Item::Size(1), Item::Size(2)];

impl Toolbars {
    // bars sit above and below the selection, clamped to the screen when they don't fit
    pub fn layout(selection: IntRect, (sw, sh): (u32, u32), swatches: usize) -> Self {
        let (sw, sh) = (sw as f32, sh as f32);
        let bar_h = BUTTON_SIZE + 2.0 * BAR_PADDING;
        let center = selection.x() as f32 + selection.width() as f32 / 2.0;

        let top_y = selection.top() as f32 - bar_h - BAR_MARGIN;
        let top_y = if top_y >= 0.0 { top_y } else { BAR_MARGIN };
        let bottom_y = selection.bottom() as f32 + BAR_MARGIN;
        let bottom_y = if bottom_y + bar_h <= sh {
            bottom_y
        } else {
            sh - bar_h - BAR_MARGIN
        };

        let tools = TOOL_KEYS.iter().map(|(_, t)| Item::Tool(*t));
        let top: Vec<Item> = tools.chain(TOP_ITEMS).collect();
        let bottom: Vec<Item> = (0..swatches)
            .map(Item::Swatch)
            .chain([Item::Custom])
            .chain(SIZE_ITEMS)
            .chain([Item::FactorDown, Item::FactorUp, Item::Fill])
            .collect();

        let (top_bar, mut buttons) = row(&top, top_y, center, sw);
        let (bottom_bar, more) = row(&bottom, bottom_y, center, sw);
        buttons.extend(more);
        Self {
            bars: [top_bar, bottom_bar],
            buttons,
        }
    }

    pub fn hit(&self, p: Point) -> Option<Item> {
        self.buttons
            .iter()
            .find(|b| b.area.contains(p))
            .map(|b| b.item)
    }

    pub fn covers(&self, p: Point) -> bool {
        self.bars.iter().any(|a| a.contains(p))
    }

    pub fn draw(&self, pm: &mut Pixmap, fonts: &Fonts, active: &Active) {
        for bar in self.bars {
            fill(pm, bar, BAR_RADIUS, BAR_COLOR);
        }
        for button in &self.buttons {
            let area = button.area;
            if is_active(button.item, active) {
                fill(pm, area, BUTTON_RADIUS, BUTTON_ACTIVE_COLOR);
            }
            if let Item::Swatch(i) = button.item {
                swatch(pm, area, active.palette[i]);
            } else if button.item == Item::Custom && !active.palette.contains(&active.color) {
                swatch(pm, area, active.color);
            } else {
                draw_label(pm, fonts, area.center(), button.item);
            }
        }
    }
}

fn row(items: &[Item], y: f32, center: f32, screen_w: f32) -> (Area, Vec<Button>) {
    let n = items.len() as f32;
    let width = n * BUTTON_SIZE + (n - 1.0) * BUTTON_GAP;
    let max_x = (screen_w - width - BAR_MARGIN).max(BAR_MARGIN);
    let x0 = (center - width / 2.0).clamp(BAR_MARGIN, max_x);
    let buttons = items
        .iter()
        .enumerate()
        .map(|(i, &item)| {
            let x = x0 + i as f32 * (BUTTON_SIZE + BUTTON_GAP);
            Button {
                area: Area {
                    x,
                    y,
                    w: BUTTON_SIZE,
                    h: BUTTON_SIZE,
                },
                item,
            }
        })
        .collect();
    let bar = Area {
        x: x0 - BAR_PADDING,
        y: y - BAR_PADDING,
        w: width + 2.0 * BAR_PADDING,
        h: BUTTON_SIZE + 2.0 * BAR_PADDING,
    };
    (bar, buttons)
}

fn is_active(item: Item, active: &Active) -> bool {
    match item {
        Item::Tool(t) => t == active.tool,
        Item::Swatch(i) => active.palette[i] == active.color,
        Item::Custom => !active.palette.contains(&active.color),
        Item::Size(i) => i == active.size,
        Item::Fill => active.fill,
        _ => false,
    }
}

fn fill(pm: &mut Pixmap, area: Area, radius: f32, color: Color) {
    if let Some(path) = area.rect().and_then(|r| tools::rounded_rect(r, radius)) {
        pm.fill_path(
            &path,
            &color.paint(),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn swatch(pm: &mut Pixmap, area: Area, color: Color) {
    let inner = Area {
        x: area.x + SWATCH_INSET,
        y: area.y + SWATCH_INSET,
        w: area.w - 2.0 * SWATCH_INSET,
        h: area.h - 2.0 * SWATCH_INSET,
    };
    fill(pm, inner, BUTTON_RADIUS, color);
}

fn draw_label(pm: &mut Pixmap, fonts: &Fonts, center: Point, item: Item) {
    let glyph = icon(item)
        .zip(fonts.icons.as_ref())
        .filter(|(g, face)| face.has_glyph(*g));
    if let Some((g, icons)) = glyph {
        tools::draw_centered(
            pm,
            icons,
            &g.to_string(),
            center,
            ICON_PX,
            BUTTON_TEXT_COLOR,
        );
    } else {
        let text = label(item);
        tools::draw_centered(pm, &fonts.text, text, center, LABEL_PX, BUTTON_TEXT_COLOR);
    }
}

fn icon(item: Item) -> Option<char> {
    Some(match item {
        Item::Tool(tool) => TOOL_ICONS[tool as usize],
        Item::Undo => ICON_UNDO,
        Item::Redo => ICON_REDO,
        Item::Copy => ICON_COPY,
        Item::Save => ICON_SAVE,
        Item::Custom => ICON_CUSTOM_COLOR,
        Item::Fill => ICON_FILL,
        _ => return None,
    })
}

fn label(item: Item) -> &'static str {
    match item {
        Item::Tool(tool) => TOOL_LABELS[tool as usize],
        Item::Undo => LABEL_UNDO,
        Item::Redo => LABEL_REDO,
        Item::Copy => LABEL_COPY,
        Item::Save => LABEL_SAVE,
        Item::Custom => LABEL_CUSTOM_COLOR,
        Item::Fill => LABEL_FILL,
        Item::FactorDown => LABEL_FACTOR_DOWN,
        Item::FactorUp => LABEL_FACTOR_UP,
        Item::Size(i) => SIZE_LABELS[i],
        Item::Swatch(_) => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_fit_on_screen() {
        let sel = IntRect::from_xywh(0, 0, 1920, 1080).unwrap();
        let bars = Toolbars::layout(sel, (1920, 1080), 10);
        for bar in bars.bars {
            assert!(bar.y >= 0.0 && bar.y + bar.h <= 1080.0);
            assert!(bar.x >= 0.0 && bar.x + bar.w <= 1920.0);
        }
    }

    #[test]
    fn hit_first_button() {
        let sel = IntRect::from_xywh(400, 400, 600, 300).unwrap();
        let bars = Toolbars::layout(sel, (1920, 1080), 10);
        let first = &bars.buttons[0];
        assert_eq!(
            bars.hit(first.area.center()),
            Some(Item::Tool(Tool::Pointer))
        );
    }
}
