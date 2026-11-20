use tiny_skia::{LineCap, Paint};

pub const OPAQUE: u8 = 255;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: OPAQUE }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    pub fn paint(self) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color_rgba8(self.r, self.g, self.b, self.a);
        paint.anti_alias = true;
        paint
    }

    // zenity prints rgb(...), configs use #rrggbb or #rrggbbaa
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix('#') {
            let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
            let color = Self::rgb(byte(0)?, byte(2)?, byte(4)?);
            return Some(if hex.len() == 8 {
                color.with_alpha(byte(6)?)
            } else {
                color
            });
        }
        let inner = s.strip_prefix("rgb")?.trim_start_matches('a');
        let inner = inner.strip_prefix('(')?.strip_suffix(')')?;
        let mut parts = inner.split(',').map(|p| p.trim().parse::<u8>().ok());
        Some(Self::rgb(parts.next()??, parts.next()??, parts.next()??))
    }
}

#[derive(Clone, Copy)]
pub struct Style {
    pub color: Color,
    pub width: f32,
    pub fill: bool,
    pub radius: f32,
    pub round_caps: bool,
}

impl Style {
    // caps of the line and arrow tools
    pub fn line_cap(self) -> LineCap {
        if self.round_caps {
            LineCap::Round
        } else {
            LineCap::Butt
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_formats() {
        assert_eq!(Color::parse("rgb(1,2,3)\n"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(
            Color::parse("rgba(1, 2, 3, 0.5)"),
            Some(Color::rgb(1, 2, 3))
        );
        assert_eq!(Color::parse("#0a0B0c"), Some(Color::rgb(10, 11, 12)));
        assert_eq!(
            Color::parse("#0a0B0c80"),
            Some(Color::rgb(10, 11, 12).with_alpha(128))
        );
        assert_eq!(Color::parse("blue"), None);
    }
}
