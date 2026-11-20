use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use anyhow::{Context, Result, anyhow};
use fontdue::{Font, FontSettings, Metrics};

use crate::{
    config::{DEFAULT_FONT_FAMILY, FACE_INDEX_MASK, FONT_QUERY, ICON_FONT_MARK, ICON_FONT_QUERY},
    util::{notify, stdout_of},
};

type Glyph = Rc<(Metrics, Vec<u8>)>;

// where a glyph is looked for once the configured fonts lack it
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    ConfiguredOnly,
    AnySystemFont,
}

// A primary font plus fallbacks. A glyph comes from the first font that has it.
pub struct Face {
    loaded: RefCell<Vec<Rc<Font>>>,
    // fallback queries, loaded on the first glyph the loaded fonts lack
    pending: RefCell<VecDeque<String>>,
    // font files already parsed, so none is parsed twice
    tried: RefCell<HashSet<String>>,
    lookup: Lookup,
    // also remembers the chars no font has
    chosen: RefCell<HashMap<char, Option<Rc<Font>>>>,
    cache: RefCell<HashMap<(char, u32), Option<Glyph>>>,
}

impl Face {
    fn new(primary: Loaded, fallback: Vec<String>, lookup: Lookup) -> Self {
        Self {
            loaded: RefCell::new(vec![primary.font]),
            pending: RefCell::new(fallback.into()),
            tried: RefCell::new(HashSet::from([primary.path])),
            lookup,
            chosen: RefCell::default(),
            cache: RefCell::default(),
        }
    }

    pub fn glyph(&self, ch: char, px: f32) -> Option<Glyph> {
        let key = (ch, px.to_bits());
        if let Some(cached) = self.cache.borrow().get(&key) {
            return cached.clone();
        }
        let glyph = self
            .font_for(ch)
            .map(|font| Rc::new(font.rasterize(ch, px)));
        self.cache.borrow_mut().insert(key, glyph.clone());
        glyph
    }

    // glyphs no font has take no room and are not drawn
    pub fn advance(&self, ch: char, px: f32) -> f32 {
        self.font_for(ch)
            .map_or(0.0, |font| font.metrics(ch, px).advance_width)
    }

    pub fn ascent(&self, px: f32) -> f32 {
        self.primary_metrics(px, |m| m.ascent)
    }

    pub fn line_height(&self, px: f32) -> f32 {
        self.primary_metrics(px, |m| m.new_line_size)
    }

    // only the fonts loaded so far, which is all the icon face has
    pub fn has_glyph(&self, ch: char) -> bool {
        pick(&self.loaded.borrow(), ch).is_some()
    }

    fn primary_metrics(&self, px: f32, pick: impl Fn(fontdue::LineMetrics) -> f32) -> f32 {
        let loaded = self.loaded.borrow();
        loaded
            .first()
            .and_then(|font| font.horizontal_line_metrics(px))
            .map_or(px, pick)
    }

    fn font_for(&self, ch: char) -> Option<Rc<Font>> {
        if let Some(found) = self.chosen.borrow().get(&ch) {
            return found.clone();
        }
        let found = self.find(ch);
        self.chosen.borrow_mut().insert(ch, found.clone());
        found
    }

    fn find(&self, ch: char) -> Option<Rc<Font>> {
        if let Some(font) = pick(&self.loaded.borrow(), ch) {
            return Some(font);
        }
        loop {
            let next = self.pending.borrow_mut().pop_front();
            let Some(query) = next else { break };
            if let Some(font) = self.add(&query).filter(|f| f.lookup_glyph_index(ch) != 0) {
                return Some(font);
            }
        }
        if self.lookup == Lookup::ConfiguredOnly {
            return None;
        }
        self.add(&charset_query(ch))
            .filter(|f| f.lookup_glyph_index(ch) != 0)
    }

    // loads the font a query resolves to, unless that file was already parsed
    fn add(&self, query: &str) -> Option<Rc<Font>> {
        let (path, index) = match resolve(query) {
            Ok(found) => found,
            Err(e) => {
                notify(false, "shotr", &format!("font {query}: {e:#}"), None);
                return None;
            }
        };
        if !self.tried.borrow_mut().insert(path.clone()) {
            return None;
        }
        let font = match parse(&path, index) {
            Ok(font) => Rc::new(font),
            Err(e) => {
                notify(false, "shotr", &format!("font {query}: {e:#}"), None);
                return None;
            }
        };
        self.loaded.borrow_mut().push(Rc::clone(&font));
        Some(font)
    }
}

struct Loaded {
    font: Rc<Font>,
    path: String,
}

// first font of the chain that has the glyph
fn pick(chain: &[Rc<Font>], ch: char) -> Option<Rc<Font>> {
    chain
        .iter()
        .find(|font| font.lookup_glyph_index(ch) != 0)
        .cloned()
}

fn charset_query(ch: char) -> String {
    format!(":charset={:x}", u32::from(ch))
}

// (file, index inside the file), as a .ttc holds several fonts
fn resolve(query: &str) -> Result<(String, u32)> {
    let out = stdout_of("fc-match", &["-f", "%{file}\n%{index}", query])?;
    let out = String::from_utf8_lossy(&out);
    let (path, index) = out.split_once('\n').context("unexpected fc-match output")?;
    Ok((path.to_owned(), face_index(index)))
}

fn face_index(raw: &str) -> u32 {
    raw.trim().parse::<u32>().unwrap_or(0) & FACE_INDEX_MASK
}

fn parse(path: &str, index: u32) -> Result<Font> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {path}"))?;
    let settings = FontSettings {
        collection_index: index,
        ..FontSettings::default()
    };
    Font::from_bytes(bytes, settings).map_err(|e| anyhow!("invalid font {path}: {e}"))
}

fn load(query: &str) -> Result<Loaded> {
    let (path, index) = resolve(query)?;
    load_file(path, index)
}

fn load_file(path: String, index: u32) -> Result<Loaded> {
    let font = Rc::new(parse(&path, index)?);
    Ok(Loaded { font, path })
}

pub struct Fonts {
    pub text: Face,
    pub icons: Option<Face>,
}

impl Fonts {
    pub fn load(family: Option<&str>, style: Option<&str>, fallback: &[String]) -> Result<Self> {
        let primary = load(&font_query(family, style))?;
        let text = Face::new(primary, fallback.to_vec(), Lookup::AnySystemFont);
        Ok(Self {
            text,
            icons: load_icons(),
        })
    }
}

fn font_query(family: Option<&str>, style: Option<&str>) -> String {
    match (family, style) {
        (None, None) => FONT_QUERY.to_owned(),
        (Some(family), None) => family.to_owned(),
        (family, Some(style)) => {
            format!("{}:style={style}", family.unwrap_or(DEFAULT_FONT_FAMILY))
        }
    }
}

// without a Nerd Font the buttons fall back to text labels
fn load_icons() -> Option<Face> {
    let (path, index) = resolve(ICON_FONT_QUERY).ok()?;
    if !path.contains(ICON_FONT_MARK) {
        return None;
    }
    let icons = load_file(path, index).ok()?;
    Some(Face::new(icons, Vec::new(), Lookup::ConfiguredOnly))
}

#[cfg(test)]
mod tests {
    use super::*;

    // None when fontconfig has nothing to offer, so the tests can skip
    fn sans() -> Option<Loaded> {
        load(FONT_QUERY).ok()
    }

    #[test]
    fn query_combines_family_and_style() {
        assert_eq!(font_query(None, None), FONT_QUERY);
        assert_eq!(font_query(Some("Roboto"), None), "Roboto");
        assert_eq!(
            font_query(Some("Roboto"), Some("Bold")),
            "Roboto:style=Bold"
        );
        assert_eq!(font_query(None, Some("Bold")), "sans:style=Bold");
    }

    #[test]
    fn face_index_drops_the_variable_instance_bits() {
        assert_eq!(face_index("2\n"), 2);
        assert_eq!(face_index("65538"), 2);
        assert_eq!(face_index("garbage"), 0);
    }

    #[test]
    fn charset_query_uses_hex() {
        assert_eq!(charset_query('\u{1f600}'), ":charset=1f600");
    }

    #[test]
    fn glyph_comes_from_the_first_font_that_has_it() {
        let Some(sans) = sans() else { return };
        assert!(pick(std::slice::from_ref(&sans.font), 'a').is_some());
        assert!(pick(std::slice::from_ref(&sans.font), '\u{10ffff}').is_none());
        let face = Face::new(sans, Vec::new(), Lookup::ConfiguredOnly);
        assert!(face.glyph('a', 20.0).is_some());
        assert!(
            face.glyph('\u{10ffff}', 20.0).is_none(),
            "missing glyphs are skipped"
        );
        assert!(face.advance('\u{10ffff}', 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fallback_loads_lazily_and_covers_what_the_primary_lacks() {
        let Some(sans) = sans() else { return };
        let icon = '\u{f245}';
        if sans.font.lookup_glyph_index(icon) != 0 || load(ICON_FONT_QUERY).is_err() {
            return;
        }
        let face = Face::new(
            sans,
            vec![ICON_FONT_QUERY.to_owned()],
            Lookup::ConfiguredOnly,
        );
        assert_eq!(
            face.loaded.borrow().len(),
            1,
            "nothing is loaded before a miss"
        );
        assert!(face.glyph('a', 20.0).is_some());
        assert_eq!(
            face.loaded.borrow().len(),
            1,
            "a hit in the primary loads nothing"
        );
        if face.glyph(icon, 20.0).is_some() {
            assert_eq!(face.loaded.borrow().len(), 2);
        }
    }

    #[test]
    fn a_font_file_is_parsed_once() {
        let Some(sans) = sans() else { return };
        let same = FONT_QUERY.to_owned();
        let face = Face::new(sans, vec![same], Lookup::ConfiguredOnly);
        assert!(face.glyph('\u{10ffff}', 20.0).is_none());
        assert_eq!(face.loaded.borrow().len(), 1);
    }
}
