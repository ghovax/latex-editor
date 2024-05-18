use std::{collections::HashMap, sync::Arc};

use crate::{attributes::Attributes, shape_plan_cache::ShapePlanCache};

use rustybuzz::Face as RustybuzzFace;
self_cell::self_cell!(
    pub struct OwnedFace {
        owner: Arc<dyn AsRef<[u8]> + Send + Sync>,

        #[covariant]
        dependent: RustybuzzFace,
    }
);

pub struct Font {
    pub swash_cache_key: (u32, swash::CacheKey),
    pub face: OwnedFace,
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub id: fontdb::ID,
    pub unicode_codepoints: Vec<u32>,
}

impl Font {
    pub fn new(database: &fontdb::Database, font_id: fontdb::ID) -> Option<Self> {
        let face_info = database.face(font_id)?;

        let unicode_codepoints = {
            database.with_face_data(font_id, |font_data, face_index| {
                let face = rustybuzz::ttf_parser::Face::parse(font_data, face_index).ok()?;
                let mut unicode_codepoints = Vec::new();

                face.tables()
                    .cmap?
                    .subtables
                    .into_iter()
                    .filter(|subtable| subtable.is_unicode())
                    .for_each(|subtable| {
                        unicode_codepoints.reserve(1024);

                        subtable.codepoints(|code_point| {
                            if subtable.glyph_index(code_point).is_some() {
                                unicode_codepoints.push(code_point);
                            }
                        });
                    });

                unicode_codepoints.shrink_to_fit();

                Some(unicode_codepoints)
            })?
        }?;

        let data = match &face_info.source {
            fontdb::Source::Binary(data) => Arc::clone(data),
            fontdb::Source::File(path) => {
                log::warn!("Unsupported `fontdb::Source::File({:?})`", path.display());
                return None;
            }
            fontdb::Source::SharedFile(_path, data) => Arc::clone(data),
        };

        Some(Self {
            id: face_info.id,
            unicode_codepoints,
            swash_cache_key: {
                let swash = swash::FontRef::from_index((*data).as_ref(), face_info.index as usize)?;
                (swash.offset, swash.key)
            },
            face: OwnedFace::try_new(Arc::clone(&data), |data| {
                RustybuzzFace::from_slice((**data).as_ref(), face_info.index).ok_or(())
            })
            .ok()?,
            data,
        })
    }

    pub fn data(&self) -> &[u8] {
        (*self.data).as_ref()
    }

    pub fn as_font_reference(&self) -> swash::FontRef<'_> {
        let swash_cache_key = &self.swash_cache_key;
        swash::FontRef {
            data: self.data(),
            offset: swash_cache_key.0,
            key: swash_cache_key.1,
        }
    }
}

/// Font-specific part of [`Attributes`] to be used for matching.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FontMatchAttributes {
    pub family: fontdb::Family<'static>,
    pub stretch: fontdb::Stretch,
    pub style: fontdb::Style,
    pub weight: fontdb::Weight,
}

impl From<Attributes> for FontMatchAttributes {
    fn from(attributes: Attributes) -> Self {
        let Attributes {
            family,
            stretch,
            style,
            weight,
            ..
        } = attributes;

        Self {
            family,
            stretch,
            style,
            weight,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontMatchKey {
    pub font_weight_difference: u16,
    pub font_weight: u16,
    pub id: fontdb::ID,
}

/// Access to the system fonts.
pub struct FontSystem {
    /// The locale of the system.
    pub locale: String,
    /// The underlying font database.
    pub database: fontdb::Database,
    /// Cache for loaded fonts from the database.
    pub font_cache: HashMap<fontdb::ID, Option<Arc<Font>>>,
    /// Cache for font matches.
    pub font_matches_cache: HashMap<FontMatchAttributes, Arc<Vec<FontMatchKey>>>,
    /// Cache for `rustybuzz` shape plans.
    pub shape_plan_cache: ShapePlanCache,
}

impl FontSystem {
    /// Creates a new font system.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let locale = sys_locale::get_locale().unwrap_or_else(|| {
            log::warn!("Failed to get system locale, falling back to en-US");
            String::from("en-US")
        });

        let mut database = fontdb::Database::new();

        // TODO(ghovax): The user might want to load additional fonts.
        database.set_monospace_family("CMU Typewriter Text");
        database.set_sans_serif_family("CMU Sans Serif");
        database.set_serif_family("CMU Serif");

        database.load_system_fonts();
        log::debug!("Parsed {} font faces", database.len(),);

        Self {
            locale,
            database,
            font_cache: HashMap::new(),
            font_matches_cache: HashMap::new(),
            shape_plan_cache: ShapePlanCache::default(),
        }
    }

    /// Get a font from the database by its ID.
    pub fn get_font(&mut self, font_id: fontdb::ID) -> Option<Arc<Font>> {
        self.font_cache
            .entry(font_id)
            .or_insert_with(|| {
                unsafe {
                    self.database.make_shared_face_data(font_id);
                }
                match Font::new(&self.database, font_id) {
                    Some(font) => Some(Arc::new(font)),
                    None => {
                        log::warn!(
                            "Failed to load the font {:?} from the database",
                            self.database.face(font_id)?.post_script_name
                        );
                        None
                    }
                }
            })
            .clone()
    }

    pub fn get_font_matches(&mut self, attributes: Attributes) -> Arc<Vec<FontMatchKey>> {
        // Clear the cache first if it reached the size limit
        if self.font_matches_cache.len() >= 1024 {
            log::debug!("Cleared the font match cache");
            self.font_matches_cache.clear();
        }

        self.font_matches_cache
            .entry(attributes.into())
            .or_insert_with(|| {
                let mut font_match_keys = self
                    .database
                    .faces()
                    .filter(|face| attributes.matches(face))
                    .map(|face| FontMatchKey {
                        font_weight_difference: attributes.weight.0.abs_diff(face.weight.0),
                        font_weight: face.weight.0,
                        id: face.id,
                    })
                    .collect::<Vec<_>>();

                // Sort so we get the keys with weight_offset = 0 first
                font_match_keys.sort();

                Arc::new(font_match_keys)
            })
            .clone()
    }
}
