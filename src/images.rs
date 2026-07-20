//! Decoding embedded image bytes into egui textures, with a small cache keyed by
//! card id so we only upload to the GPU once per image.

use crate::model::CardId;
use std::collections::HashMap;

/// Cached texture plus the byte-length it was built from, so we can detect when
/// a card's image was replaced and rebuild it.
pub struct TextureCache {
    entries: HashMap<CardId, (usize, egui::TextureHandle)>,
}

impl Default for TextureCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl TextureCache {
    /// Return a texture for the given card's image bytes, decoding and uploading
    /// on first use (or after the bytes change). `None` if decoding fails.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        card: CardId,
        bytes: &[u8],
    ) -> Option<egui::TextureHandle> {
        if let Some((len, tex)) = self.entries.get(&card) {
            if *len == bytes.len() {
                return Some(tex.clone());
            }
        }
        let image = decode(bytes)?;
        let tex = ctx.load_texture(format!("card-image-{card}"), image, Default::default());
        self.entries.insert(card, (bytes.len(), tex.clone()));
        Some(tex)
    }

    pub fn forget(&mut self, card: CardId) {
        self.entries.remove(&card);
    }
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        img.as_flat_samples().as_slice(),
    ))
}
