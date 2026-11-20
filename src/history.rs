use crate::tools::Shape;

enum Edit {
    Insert(usize, Shape),
    Remove(usize),
    Translate(usize, (f32, f32)),
    Clear,
    Restore(Vec<Shape>),
}

#[derive(Default)]
pub struct History {
    shapes: Vec<Shape>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    version: u64,
}

impl History {
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    // bumped on every change so callers can cache what they drew
    pub fn version(&self) -> u64 {
        self.version
    }

    // live move while dragging; the undo entry is recorded on release
    pub fn nudge(&mut self, index: usize, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if let Some(shape) = self.shapes.get_mut(index) {
            shape.translate(dx, dy);
            self.version += 1;
        }
    }

    pub fn add(&mut self, shape: Shape) {
        self.run(Edit::Insert(self.shapes.len(), shape));
    }

    pub fn remove(&mut self, index: usize) {
        self.run(Edit::Remove(index));
    }

    pub fn clear(&mut self) {
        if !self.shapes.is_empty() {
            self.run(Edit::Clear);
        }
    }

    // the shape was already moved live, only record the undo
    pub fn record_move(&mut self, index: usize, total: (f32, f32)) {
        self.undo.push(Edit::Translate(index, (-total.0, -total.1)));
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        if let Some(edit) = self.undo.pop() {
            let inverse = self.apply(edit);
            self.redo.push(inverse);
        }
    }

    pub fn redo(&mut self) {
        if let Some(edit) = self.redo.pop() {
            let inverse = self.apply(edit);
            self.undo.push(inverse);
        }
    }

    pub fn next_marker(&self) -> u32 {
        let max = self.shapes.iter().filter_map(Shape::marker_number).max();
        max.map_or(1, |n| n + 1)
    }

    fn run(&mut self, edit: Edit) {
        let inverse = self.apply(edit);
        self.undo.push(inverse);
        self.redo.clear();
    }

    fn apply(&mut self, edit: Edit) -> Edit {
        self.version += 1;
        match edit {
            Edit::Insert(i, shape) => {
                self.shapes.insert(i, shape);
                Edit::Remove(i)
            }
            Edit::Remove(i) => Edit::Insert(i, self.shapes.remove(i)),
            Edit::Translate(i, (dx, dy)) => {
                if let Some(shape) = self.shapes.get_mut(i) {
                    shape.translate(dx, dy);
                }
                Edit::Translate(i, (-dx, -dy))
            }
            Edit::Clear => Edit::Restore(std::mem::take(&mut self.shapes)),
            Edit::Restore(shapes) => {
                self.shapes = shapes;
                Edit::Clear
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tiny_skia::Point;

    use super::*;
    use crate::{config::PALETTE, style::Style};

    fn marker(number: u32) -> Shape {
        let style = Style {
            color: PALETTE[0],
            width: 1.0,
            fill: false,
            radius: 0.0,
            round_caps: true,
        };
        Shape::Marker {
            pos: Point::zero(),
            number,
            style,
        }
    }

    #[test]
    fn marker_numbers() {
        let mut h = History::default();
        assert_eq!(h.next_marker(), 1);
        h.add(marker(h.next_marker()));
        h.add(marker(h.next_marker()));
        assert_eq!(h.next_marker(), 3);
        h.undo();
        assert_eq!(h.next_marker(), 2);
    }

    #[test]
    fn undo_redo() {
        let mut h = History::default();
        h.add(marker(1));
        h.add(marker(2));
        h.undo();
        h.undo();
        assert!(h.shapes().is_empty());
        h.redo();
        assert_eq!(h.shapes().len(), 1);
        h.add(marker(5));
        h.redo();
        assert_eq!(h.shapes().len(), 2, "a new edit clears redo");
    }

    #[test]
    fn version_changes_with_every_edit() {
        let mut h = History::default();
        let start = h.version();
        h.add(marker(1));
        let added = h.version();
        h.undo();
        assert!(start < added && added < h.version());
    }

    #[test]
    fn zero_nudge_keeps_the_version() {
        let mut h = History::default();
        h.add(marker(1));
        let before = h.version();
        h.nudge(0, 0.0, 0.0);
        assert_eq!(h.version(), before);
        h.nudge(0, 1.0, 0.0);
        assert!(h.version() > before);
    }

    #[test]
    fn clear_and_remove() {
        let mut h = History::default();
        h.add(marker(1));
        h.add(marker(2));
        h.remove(0);
        assert_eq!(h.shapes().len(), 1);
        h.clear();
        assert!(h.shapes().is_empty());
        h.undo();
        h.undo();
        assert_eq!(h.shapes().len(), 2);
    }
}
