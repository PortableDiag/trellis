//! Colour emoji, painted over the glyphs egui has already laid out.
//!
//! ## Why this shape
//!
//! egui rasterizes glyph **outlines** through `ab_glyph`. Every colour-emoji
//! font is a different thing: `NotoColorEmoji` is CBDT/CBLC (embedded PNGs),
//! Apple Color Emoji is `sbix` (also bitmaps), Segoe UI Emoji is COLR/CPAL
//! (layered vectors). None of them is an outline the text stack can draw, which
//! is why "just add the colour font" produces blank glyphs rather than colour —
//! a silent failure, and the reason this was recorded as not worth doing.
//!
//! The way through is to stop asking the text stack for colour. egui has
//! already decided *where* every glyph goes; we read those positions back at
//! the end of the frame and paint a textured quad over each emoji.
//!
//! Three properties make that honest rather than a hack:
//!
//! * **Layout is untouched.** The vendored monochrome `NotoEmoji` still supplies
//!   the advance width, so wrapping, selection and hit-testing are exactly what
//!   they were. We only draw.
//! * **It degrades to today.** No colour font on the machine, or a font whose
//!   glyphs are COLR rather than bitmaps (Windows), and nothing is painted — you
//!   get the monochrome glyph that is already there, not a hole.
//! * **It reaches everything.** Scanning the frame's paint lists covers markdown
//!   bodies, card titles, the tree, panels and menus alike. Hooking each call
//!   site instead would have produced exactly the inconsistent, partial set that
//!   made this look not worth building.
//!
//! ## What it does not do
//!
//! A ZWJ sequence (👨‍👩‍👧) lays out as its component glyphs, so it paints as its
//! components. Doing better means shaping, which egui does not do here either.

use std::collections::HashMap;

use egui::{Color32, Context, LayerId, Rect, Shape, TextureHandle, TextureOptions};

/// Where a colour-emoji font lives, per platform. Checked in order; the first
/// readable one wins.
const FONT_PATHS: &[&str] = &[
    // Linux
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/local/share/fonts/NotoColorEmoji.ttf",
    // macOS — `sbix`, which ttf-parser reads the same way as CBDT.
    "/System/Library/Fonts/Apple Color Emoji.ttc",
    // Windows ships Segoe UI Emoji, which is COLR/CPAL: no raster image per
    // glyph, so it is listed for completeness and will simply find nothing.
    "C:\\Windows\\Fonts\\seguiemj.ttf",
];

/// Environment override, so a machine with the font somewhere unusual — or a
/// user who wants a different emoji set — can point at it without a rebuild.
const FONT_ENV: &str = "TRELLIS_EMOJI_FONT";

pub struct Emoji {
    /// The font file, kept resident: `ttf_parser::Face` borrows its bytes, and
    /// re-parsing per lookup is cheap because the tables are read lazily.
    font: Option<Vec<u8>>,
    /// One texture per character. `None` records "asked, not available", so a
    /// character the font lacks costs one lookup, not one per frame.
    cache: HashMap<char, Option<TextureHandle>>,
    /// Which file the colour glyphs come from, for Settings to report. A
    /// missing font makes emoji silently stay monochrome, which reads as the
    /// feature being broken rather than absent — so it has to be answerable.
    source: Option<String>,
}

impl Default for Emoji {
    fn default() -> Self {
        Self::new()
    }
}

impl Emoji {
    pub fn new() -> Self {
        let from_env = std::env::var(FONT_ENV).ok().filter(|p| !p.is_empty());
        let found = from_env
            .and_then(|p| std::fs::read(&p).ok().map(|b| (p, b)))
            .or_else(|| {
                FONT_PATHS
                    .iter()
                    .find_map(|p| std::fs::read(p).ok().map(|b| ((*p).to_owned(), b)))
            });
        let (source, font) = match found {
            Some((path, bytes)) => (Some(path), Some(bytes)),
            None => (None, None),
        };
        Self {
            source,
            font,
            cache: HashMap::new(),
        }
    }

    /// One line for Settings: where the colour glyphs come from, or why there
    /// are none.
    pub fn status(&self) -> String {
        match &self.source {
            Some(path) => format!("Colour emoji: {path}"),
            None => format!(
                "Colour emoji: no emoji font found — emoji stay monochrome. \
                 Install Noto Color Emoji, or set {FONT_ENV} to a font file."
            ),
        }
    }

    /// Which characters we take responsibility for.
    ///
    /// Deliberately narrow. DejaVu draws arrows, dashes and box-drawing better
    /// than an emoji font does, and those live in the same neighbourhoods — so
    /// this covers the emoji planes plus the two BMP blocks that actually hold
    /// pictographs (Misc Symbols/Dingbats, and Misc Symbols & Arrows for the
    /// coloured squares). Anything else keeps rendering as it does today.
    fn is_emoji(ch: char) -> bool {
        let c = ch as u32;
        (0x1F000..=0x1FAFF).contains(&c)      // emoji planes proper
            || (0x2600..=0x27BF).contains(&c) // ☀ ✅ ✈ …
            || (0x2B00..=0x2BFF).contains(&c) // ⬛ ⭐ …
            || (0x1F1E6..=0x1F1FF).contains(&c) // regional indicators (flags)
    }

    fn texture(&mut self, ctx: &Context, ch: char) -> Option<TextureHandle> {
        if let Some(hit) = self.cache.get(&ch) {
            return hit.clone();
        }
        let handle = self.load(ctx, ch);
        self.cache.insert(ch, handle.clone());
        handle
    }

    fn load(&self, ctx: &Context, ch: char) -> Option<TextureHandle> {
        let bytes = self.font.as_ref()?;
        // A .ttc holds several faces; index 0 is the emoji face in the ones we
        // look at, and a plain .ttf ignores the index.
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let gid = face.glyph_index(ch)?;
        // u16::MAX asks for the largest strike the font has: these are scaled
        // down to text size, and scaling a small bitmap up looks like a mistake.
        let raster = face.glyph_raster_image(gid, u16::MAX)?;
        let decoded = image::load_from_memory(raster.data).ok()?;
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        Some(ctx.load_texture(
            format!("emoji-{:x}", ch as u32),
            image,
            TextureOptions::LINEAR,
        ))
    }

    /// Paint colour over every emoji drawn this frame.
    ///
    /// Call at the very end of `update`, after everything has been drawn:
    /// anything added to a paint list afterwards is not covered, and anything
    /// drawn on top of the overlay hides it.
    pub fn overlay(&mut self, ctx: &Context) {
        if self.font.is_none() {
            return;
        }

        // Phase 1: read. Collect what needs painting without holding the
        // graphics lock while loading textures, which takes a different lock.
        let mut layer_ids: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
        // Panels and the canvas draw into the background layer, which is not an
        // "area" and so is absent from that list — i.e. most of this app.
        layer_ids.push(LayerId::background());

        let mut todo: Vec<(LayerId, Rect, Rect, char)> = Vec::new();
        ctx.graphics_mut(|layers| {
            for &layer_id in &layer_ids {
                let Some(list) = layers.get(layer_id) else {
                    continue;
                };
                for entry in list.all_entries() {
                    collect(&entry.shape, entry.clip_rect, layer_id, &mut todo);
                }
            }
        });
        if todo.is_empty() {
            return;
        }

        // Phase 2: resolve characters to textures.
        let mut shapes: Vec<(LayerId, Rect, Shape)> = Vec::new();
        for (layer_id, clip, rect, ch) in todo {
            if let Some(tex) = self.texture(ctx, ch) {
                shapes.push((
                    layer_id,
                    clip,
                    Shape::image(tex.id(), rect, uv_full(), Color32::WHITE),
                ));
            }
        }

        // Phase 3: draw, into the same layer and under the same clip rect as the
        // text it covers — otherwise an emoji in a scroll area paints outside it.
        ctx.graphics_mut(|layers| {
            for (layer_id, clip, shape) in shapes {
                layers.entry(layer_id).add(clip, shape);
            }
        });
    }
}

fn uv_full() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

/// Walk a shape for emoji glyphs. Shapes nest (`Shape::Vec`), so this recurses
/// rather than matching only the top level.
fn collect(shape: &Shape, clip: Rect, layer_id: LayerId, out: &mut Vec<(LayerId, Rect, Rect, char)>) {
    match shape {
        Shape::Vec(shapes) => {
            for s in shapes {
                collect(s, clip, layer_id, out);
            }
        }
        Shape::Text(text) => {
            // Rotated text would need the quad rotated with it; it is rare
            // enough (and emoji in it rarer) that skipping is honest, where
            // painting an unrotated square over rotated text would not be.
            if text.angle != 0.0 {
                return;
            }
            for row in &text.galley.rows {
                for glyph in &row.glyphs {
                    if !Emoji::is_emoji(glyph.chr) {
                        continue;
                    }
                    // `logical_rect` is the box the layout reserved: advance
                    // width by line height. An emoji bitmap is square, so fit a
                    // square inside it rather than stretching to that ratio.
                    let logical = glyph.logical_rect().translate(text.pos.to_vec2());
                    let side = logical.width().min(logical.height());
                    if side <= 0.0 {
                        continue;
                    }
                    let centre = logical.center();
                    let rect = Rect::from_center_size(centre, egui::vec2(side, side));
                    if !clip.intersects(rect) {
                        continue; // scrolled out of view — don't load its texture
                    }
                    out.push((layer_id, clip, rect, glyph.chr));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::Emoji;

    /// The range predicate is the whole safety property: too wide and it paints
    /// squares over the arrows and dashes DejaVu draws better (which is why the
    /// font chain puts DejaVu first in the first place), too narrow and the
    /// coloured circles someone actually reaches for stay grey.
    #[test]
    fn only_pictographs_are_claimed() {
        for ch in ['🔴', '🟢', '🟩', '✅', '⭐', '🚀', '☀', '⬛'] {
            assert!(Emoji::is_emoji(ch), "{ch} should be painted in colour");
        }
        for ch in ['→', '←', '↔', '⇒', '—', '–', '·', 'A', '1', 'é', '…', '“'] {
            assert!(!Emoji::is_emoji(ch), "{ch} belongs to the text font");
        }
    }

    /// A missing font must be inert, not a panic and not a blank frame: the
    /// overlay simply does nothing and the monochrome glyph already drawn
    /// stands. This is the Windows case as much as the no-font-installed one.
    #[test]
    fn a_missing_font_is_inert() {
        let emoji = Emoji {
            font: None,
            source: None,
            cache: Default::default(),
        };
        assert!(emoji.status().contains("no emoji font found"));
    }
}
