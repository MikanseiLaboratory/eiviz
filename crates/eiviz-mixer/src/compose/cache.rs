use crate::abi::{
    LABEL_BASE, OUTPUT_PREVIEW, OUTPUT_PROGRAM, mixing_unit_bus, mixing_unit_from_source,
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct LabelTexKey {
    text: String,
    rgb: [u8; 3],
    font_q: u16,
    dest_w: u32,
}

impl LabelTexKey {
    pub(crate) fn new(text: &str, rgb: [u8; 3], font_px: f32, dest_w: u32) -> Self {
        Self {
            text: text.to_string(),
            rgb,
            font_q: (font_px * 2.0).round() as u16,
            dest_w,
        }
    }
}

pub(crate) fn label_cache_key(key: &LabelTexKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    LABEL_BASE.wrapping_add(0xF000) ^ hasher.finish()
}

pub(crate) fn mv_label_rgb(
    source_id: u64,
    preview: [u8; 3],
    program: [u8; 3],
    inactive: [u8; 3],
) -> [u8; 3] {
    if mixing_unit_from_source(source_id).is_none() {
        return inactive;
    }
    match mixing_unit_bus(source_id) {
        OUTPUT_PREVIEW => preview,
        OUTPUT_PROGRAM => program,
        _ => inactive,
    }
}
pub(crate) fn mv_tally_program(source_id: u64, tallies: &[(u64, u64)]) -> Option<bool> {
    if mixing_unit_from_source(source_id).is_some() {
        return match mixing_unit_bus(source_id) {
            OUTPUT_PREVIEW => Some(false),
            OUTPUT_PROGRAM => Some(true),
            _ => None,
        };
    }
    if source_id != 0 && tallies.iter().any(|(_, program)| *program == source_id) {
        return Some(true);
    }
    if source_id != 0 && tallies.iter().any(|(preview, _)| *preview == source_id) {
        return Some(false);
    }
    None
}
