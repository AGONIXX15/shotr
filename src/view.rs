use tiny_skia::Point;

use crate::config::ZOOM_MAX;

// Maps between screen pixels and image pixels: screen = image * zoom + pan.
pub struct View {
    image: (f32, f32),
    screen: (f32, f32),
    fit: f32,
    zoom: f32,
    pan: (f32, f32),
}

impl View {
    pub fn new(image: (u32, u32), screen: (u32, u32)) -> Self {
        let image = (image.0 as f32, image.1 as f32);
        let screen = (screen.0 as f32, screen.1 as f32);
        let fit = (screen.0 / image.0).min(screen.1 / image.1).min(1.0);
        let mut view = Self {
            image,
            screen,
            fit,
            zoom: fit,
            pan: (0.0, 0.0),
        };
        view.clamp();
        view
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn pan(&self) -> (f32, f32) {
        self.pan
    }

    pub fn is_identity(&self) -> bool {
        (self.zoom - 1.0).abs() < f32::EPSILON
            && self.pan == (0.0, 0.0)
            && self.image == self.screen
    }

    pub fn to_image(&self, p: Point) -> Point {
        Point::from_xy(
            (p.x - self.pan.0) / self.zoom,
            (p.y - self.pan.1) / self.zoom,
        )
    }

    pub fn to_screen(&self, p: Point) -> Point {
        Point::from_xy(p.x * self.zoom + self.pan.0, p.y * self.zoom + self.pan.1)
    }

    // keeps the image point under `at` fixed
    pub fn zoom_at(&mut self, at: Point, factor: f32) {
        let before = self.to_image(at);
        self.zoom = (self.zoom * factor).clamp(self.fit, self.fit * ZOOM_MAX);
        self.pan = (at.x - before.x * self.zoom, at.y - before.y * self.zoom);
        self.clamp();
    }

    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan = (self.pan.0 + dx, self.pan.1 + dy);
        self.clamp();
    }

    fn clamp(&mut self) {
        let fit_axis = |image: f32, screen: f32, pan: f32| {
            let content = image * self.zoom;
            if content <= screen {
                // whole pixels, so a centred 1:1 image stays sharp
                ((screen - content) / 2.0).round()
            } else {
                pan.clamp(screen - content, 0.0)
            }
        };
        self.pan = (
            fit_axis(self.image.0, self.screen.0, self.pan.0),
            fit_axis(self.image.1, self.screen.1, self.pan.1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_size_starts_as_identity() {
        let view = View::new((1920, 1080), (1920, 1080));
        assert!(view.is_identity());
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut view = View::new((1920, 1080), (1920, 1080));
        let at = Point::from_xy(700.0, 400.0);
        let before = view.to_image(at);
        view.zoom_at(at, 2.0);
        let after = view.to_image(at);
        assert!((before.x - after.x).abs() < 0.01 && (before.y - after.y).abs() < 0.01);
        assert!((view.zoom() - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cannot_zoom_out_past_fit_or_pan_off_image() {
        let mut view = View::new((1920, 1080), (1920, 1080));
        view.zoom_at(Point::from_xy(0.0, 0.0), 0.1);
        assert!((view.zoom() - 1.0).abs() < f32::EPSILON);
        view.zoom_at(Point::from_xy(960.0, 540.0), 2.0);
        view.pan_by(10_000.0, 10_000.0);
        assert_eq!(view.pan(), (0.0, 0.0));
    }

    #[test]
    fn large_image_is_fitted_and_centered() {
        let view = View::new((4000, 1000), (2000, 1000));
        assert!((view.zoom() - 0.5).abs() < f32::EPSILON);
        let top_left = view.to_screen(Point::from_xy(0.0, 0.0));
        assert!((top_left.y - 250.0).abs() < 0.01);
    }

    #[test]
    fn small_image_is_centered_at_one_to_one() {
        let view = View::new((801, 600), (1920, 1080));
        assert!((view.zoom() - 1.0).abs() < f32::EPSILON);
        let corner = view.to_screen(Point::from_xy(0.0, 0.0));
        assert_eq!((corner.x, corner.y), (560.0, 240.0));
        let back = view.to_image(Point::from_xy(560.0, 240.0));
        assert_eq!((back.x, back.y), (0.0, 0.0));
        assert!(!view.is_identity());
    }
}
