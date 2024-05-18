use std::collections::{hash_map::Entry, HashMap};

use crate::font_system::Font;

/// Key for caching shape plans.
#[derive(Debug, Hash, PartialEq, Eq)]
struct ShapePlanKey {
    pub font_id: fontdb::ID,
    pub direction: rustybuzz::Direction,
    pub script: rustybuzz::Script,
    pub language: Option<rustybuzz::Language>,
}

/// A helper structure for caching rustybuzz shape plans.
#[derive(Default)]
pub struct ShapePlanCache(HashMap<ShapePlanKey, rustybuzz::ShapePlan>);

impl ShapePlanCache {
    pub fn get_from_font_and_buffer_info(
        &mut self,
        font: &Font,
        buffer: &rustybuzz::UnicodeBuffer,
    ) -> &rustybuzz::ShapePlan {
        let key = ShapePlanKey {
            font_id: font.id,
            direction: buffer.direction(),
            script: buffer.script(),
            language: buffer.language(),
        };

        match self.0.entry(key) {
            Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
            Entry::Vacant(vacant_entry) => {
                let ShapePlanKey {
                    direction,
                    script,
                    language,
                    ..
                } = vacant_entry.key();

                let plan = rustybuzz::ShapePlan::new(
                    font.face.borrow_dependent(),
                    *direction,
                    Some(*script),
                    language.as_ref(),
                    &[],
                );
                vacant_entry.insert(plan)
            }
        }
    }
}
