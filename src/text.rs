#[derive(Default)]
pub struct TextBuf {
    chars: Vec<char>,
    cursor: usize,
    // input method composition, not text until committed
    preedit: Vec<char>,
}

// A window of at most `max` bytes around `cursor`, cut on char boundaries; the cursor is
// returned relative to the window.
pub fn clip_window(text: &str, cursor: usize, max: usize) -> (&str, usize) {
    if text.len() <= max {
        return (text, cursor);
    }
    let mut end = (cursor.saturating_sub(max / 2) + max).min(text.len());
    let mut start = end - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[start..end], cursor - start)
}

// how many chars fit in `bytes` bytes
fn chars_within(chars: impl Iterator<Item = char>, bytes: usize) -> usize {
    let mut used = 0;
    chars
        .take_while(|c| {
            used += c.len_utf8();
            used <= bytes
        })
        .count()
}

impl TextBuf {
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        let starts = self.line_starts();
        let line = starts.iter().rposition(|&s| s <= self.cursor).unwrap_or(0);
        (line, self.cursor - starts[line])
    }

    pub fn shown(&self) -> String {
        let (before, after) = self.chars.split_at(self.cursor);
        before.iter().chain(&self.preedit).chain(after).collect()
    }

    // (line, first column, end column)
    pub fn preedit_span(&self) -> Option<(usize, usize, usize)> {
        if self.preedit.is_empty() {
            return None;
        }
        let (line, col) = self.cursor_pos();
        Some((line, col, col + self.preedit.len()))
    }

    pub fn caret_pos(&self) -> (usize, usize) {
        let (line, col) = self.cursor_pos();
        (line, col + self.preedit.len())
    }

    // One input method update; the delete lengths are in bytes, like the protocol sends them.
    pub fn apply_ime(&mut self, delete: (usize, usize), commit: &str, preedit: &str) {
        let before = chars_within(self.chars[..self.cursor].iter().rev().copied(), delete.0);
        let after = chars_within(self.chars[self.cursor..].iter().copied(), delete.1);
        self.preedit.clear();
        for _ in 0..before {
            self.backspace();
        }
        for _ in 0..after {
            self.delete();
        }
        self.insert(commit);
        self.preedit = preedit.chars().filter(|c| !c.is_control()).collect();
    }

    // the text and the cursor as a byte offset, for the input method
    pub fn surrounding(&self) -> (String, usize) {
        let offset = self.chars[..self.cursor].iter().map(|c| c.len_utf8()).sum();
        (self.chars.iter().collect(), offset)
    }

    pub fn clear_preedit(&mut self) {
        self.preedit.clear();
    }

    pub fn insert(&mut self, s: &str) {
        for ch in s.chars().filter(|c| !c.is_control()) {
            self.insert_char(ch);
        }
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn word_backspace(&mut self) {
        let mut start = self.cursor;
        while start > 0 && self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        let (line, _) = self.cursor_pos();
        self.cursor = self.line_starts()[line];
    }

    pub fn end(&mut self) {
        let (line, _) = self.cursor_pos();
        self.cursor = self.line_end(line);
    }

    pub fn up(&mut self) {
        let (line, col) = self.cursor_pos();
        self.cursor = match line.checked_sub(1) {
            Some(prev) => self.at_col(prev, col),
            None => 0,
        };
    }

    pub fn down(&mut self) {
        let (line, col) = self.cursor_pos();
        self.cursor = if line + 1 < self.line_starts().len() {
            self.at_col(line + 1, col)
        } else {
            self.chars.len()
        };
    }

    fn insert_char(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    fn line_starts(&self) -> Vec<usize> {
        let breaks = self.chars.iter().enumerate().filter(|(_, c)| **c == '\n');
        std::iter::once(0)
            .chain(breaks.map(|(i, _)| i + 1))
            .collect()
    }

    fn line_end(&self, line: usize) -> usize {
        self.line_starts()
            .get(line + 1)
            .map_or(self.chars.len(), |next| next - 1)
    }

    fn at_col(&self, line: usize, col: usize) -> usize {
        let start = self.line_starts()[line];
        (start + col).min(self.line_end(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuf {
        let mut b = TextBuf::default();
        for (i, line) in s.split('\n').enumerate() {
            if i > 0 {
                b.newline();
            }
            b.insert(line);
        }
        b
    }

    #[test]
    fn edits_at_cursor() {
        let mut b = buf("hola");
        b.left();
        b.backspace();
        b.insert("X");
        assert_eq!(b.surrounding().0, "hoXa");
        b.delete();
        assert_eq!(b.surrounding().0, "hoX");
    }

    #[test]
    fn word_backspace_removes_last_word() {
        let mut b = buf("one two  ");
        b.word_backspace();
        assert_eq!(b.surrounding().0, "one ");
    }

    #[test]
    fn vertical_movement_keeps_column() {
        let mut b = buf("abcd\nef\nghij");
        assert_eq!(b.cursor_pos(), (2, 4));
        b.up();
        assert_eq!(b.cursor_pos(), (1, 2));
        b.up();
        assert_eq!(b.cursor_pos(), (0, 2));
        b.down();
        b.down();
        b.home();
        assert_eq!(b.cursor_pos(), (2, 0));
        b.end();
        assert_eq!(b.cursor_pos(), (2, 4));
    }

    #[test]
    fn preedit_is_shown_but_not_part_of_the_text() {
        let mut b = buf("ab");
        b.left();
        b.apply_ime((0, 0), "", "xyz");
        assert_eq!(b.surrounding().0, "ab");
        assert_eq!(b.shown(), "axyzb");
        assert_eq!(b.preedit_span(), Some((0, 1, 4)));
        assert_eq!(b.caret_pos(), (0, 4));
        assert_eq!(b.cursor_pos(), (0, 1));
    }

    #[test]
    fn commit_replaces_the_preedit() {
        let mut b = buf("ab");
        b.apply_ime((0, 0), "", "ni");
        b.apply_ime((0, 0), "你", "");
        assert_eq!(b.surrounding().0, "ab你");
        assert_eq!(b.shown(), "ab你");
        assert_eq!(b.preedit_span(), None);
    }

    #[test]
    fn ime_can_delete_around_the_cursor() {
        let mut b = buf("abcd");
        b.left();
        b.apply_ime((1, 1), "X", "");
        assert_eq!(b.surrounding().0, "abX");
        b.clear_preedit();
    }

    #[test]
    fn preedit_drops_control_characters() {
        let mut b = TextBuf::default();
        b.apply_ime((0, 0), "", "a\nb");
        assert_eq!(b.shown(), "ab");
    }

    #[test]
    fn ime_lengths_are_bytes_not_chars() {
        let mut b = buf("a你é");
        b.apply_ime((2, 0), "", "");
        assert_eq!(b.surrounding().0, "a你");
        b.apply_ime((3, 0), "", "");
        assert_eq!(b.surrounding().0, "a");
        let mut b = buf("你好é");
        b.left();
        b.left();
        b.apply_ime((0, 3), "x", "");
        assert_eq!(b.surrounding(), ("你xé".to_owned(), 4));
    }

    #[test]
    fn a_length_that_splits_a_char_deletes_nothing() {
        let mut b = buf("你");
        b.apply_ime((2, 0), "", "");
        assert_eq!(b.surrounding().0, "你");
    }

    #[test]
    fn clip_window_stays_on_char_boundaries_around_the_cursor() {
        let text = "你好é".repeat(1500);
        let cursor = text.char_indices().nth(2000).unwrap().0;
        let (window, offset) = clip_window(&text, cursor, 4000);
        assert!(window.len() <= 4000 && window.len() > 3990);
        assert_eq!(
            window[offset..].chars().next(),
            text[cursor..].chars().next()
        );
        assert_eq!(clip_window("short", 2, 4000), ("short", 2));
        let (head, at) = clip_window(&text, 0, 4000);
        assert_eq!((at, head.len() <= 4000), (0, true));
        let (tail, at) = clip_window(&text, text.len(), 4000);
        assert_eq!(at, tail.len());
    }
}
