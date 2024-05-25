// use document::{Document, DocumentElement, EditingCursor};
use gtk4::{cairo, glib, prelude::*};
use gtk4::{DrawingArea, Window};
use skia_safe::image::CachingHint;
use skia_safe::{ColorSpace, ImageInfo, Paint, Path, PathBuilder, Pixmap, Rect, Surface};
use std::cell::RefCell;
use std::panic;
use std::rc::Rc;
use text::attributes::Attributes;
use text::{
    attributes::{Style, Weight},
    font_system::FontSystem,
    line_buffer::LineBuffer,
    swash_cache::SwashCache,
};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;

// mod document;

use std::num::NonZeroU32;

use unicode_segmentation::UnicodeSegmentation as _;

// NOTE(ghovax): This could be serialized, but I need to find a way to serialize `Attrs`.
pub enum DocumentElement {
    Line {
        anchor_point: (f32, f32),
        spans: Vec<(String, Attributes)>,
    },
}

#[derive(Default, Debug)]
pub struct EditingCursor {
    line_index: usize,
    glyph_index_in_line: usize,
}

impl EditingCursor {
    pub fn from_mouse_position(
        physically_layouted_line_buffers: &[LineBuffer],
        mouse_position: (f64, f64),
    ) -> Self {
        let mut selected_line_index = 0;
        let mut selected_glyph_index_in_line = 0;

        'outer_loop: for (line_index, line_buffer) in
            physically_layouted_line_buffers.iter().enumerate()
        {
            if line_buffer.layouted_line.is_none() {
                log::warn!("The `LineBuffer` at index {} is not layouted yet when trying to get the cursor position", line_index);
                continue;
            }

            let layouted_line = line_buffer.layouted_line.as_ref().unwrap();
            let line_height = layouted_line.maximum_y_reach + layouted_line.minimum_y_origin;

            let line_vertical_position = layouted_line
                .layouted_glyphs
                .first()
                .unwrap()
                .physical_y_offset
                .unwrap() as f64;

            if mouse_position.1 <= line_vertical_position
                && mouse_position.1 > line_vertical_position - line_height as f64
            {
                for (glyph_index, glyph) in layouted_line.layouted_glyphs.iter().enumerate() {
                    if glyph.contains_horizontal_position(mouse_position.0 as f32) {
                        selected_line_index = line_index;
                        selected_glyph_index_in_line = glyph_index;
                        break 'outer_loop;
                    }
                }
            }
        }

        Self {
            line_index: selected_line_index,
            glyph_index_in_line: selected_glyph_index_in_line,
        }
    }
}

pub struct ProgramConfiguration {}

fn main() -> glib::ExitCode {
    // Initialize the logging handler
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Enable support for high dpi displays
    // std::env::set_var("GDK_SCALE", "2");
    // std::env::set_var("GDK_DPI_SCALE", "0.5");

    let application = gtk4::Application::builder()
        .application_id("com.github.ghovax.editex")
        .build();

    application.connect_activate(|application| {
        let window = gtk4::ApplicationWindow::new(application);

        let (default_window_width, default_window_height) = (800, 600);
        window.set_default_size(default_window_width, default_window_height);
        let scale_factor = window.scale_factor();
        log::debug!("The scale factor is {}", scale_factor);

        let attributes = Attributes::new();
        // NOTE(ghovax): This could be loaded from an actual document.
        let document_elements = vec![DocumentElement::Line {
            anchor_point: (85.0, 190.0),
            spans: vec![
                ("pop".to_string(), attributes),
                ("old ".to_string(), attributes.italic()),
                ("example text ßåß√Ï√ÅÏ".to_string(), attributes.bold()),
            ],
        }];
        // NOTE(ghovax): This could be loaded from a configuration file.
        let default_font_size = 32.0;
        let mut font_size = default_font_size * scale_factor as f32;
        let mut font_system = FontSystem::new();
        let mut rasterizer_cache = SwashCache::new();
        let editing_cursor = EditingCursor::default();

        let drawing_area = DrawingArea::new();

        let surface = Surface::new_raster_n32_premul((
            default_window_width * scale_factor,
            default_window_height * scale_factor,
        ))
        .unwrap();
        log::debug!(
            "The surface was initialized with a size of {:?}",
            (surface.as_ref().width(), surface.as_ref().height())
        );
        let surface_reference = Rc::new(RefCell::new(surface));

        drawing_area.set_draw_func(move |widget, cairo_context, width, height| {
            let mut surface = surface_reference.borrow_mut();

            if surface.as_ref().width() != width * scale_factor
                || surface.as_ref().height() != height * scale_factor
            {
                let surface_replacement =
                    Surface::new_raster_n32_premul((width * scale_factor, height * scale_factor))
                        .unwrap();
                let _ = std::mem::replace(&mut *surface, surface_replacement);
                log::trace!("The surface was resized to {:?}", (width, height));
            }

            // Do all the drawing operations
            let canvas = surface.canvas();
            canvas.clear(skia_safe::Color::WHITE);

            let mut painting_options = Paint::default();
            let mut draw_filled_rectangle = |x, y, width, height, color: text::color::Color| {
                painting_options.set_color(skia_safe::Color::from_argb(
                    color.a(),
                    color.r(),
                    color.g(),
                    color.b(),
                ));
                canvas.draw_rect(
                    Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
                    &painting_options,
                );
            };
            let mut layouted_line_buffers = Vec::new();

            for document_element in document_elements.iter() {
                match document_element {
                    DocumentElement::Line {
                        anchor_point,
                        spans,
                    } => {
                        let default_attributes = Attributes::new();
                        let mut line_buffer = LineBuffer::from_rich_text(spans, default_attributes);
                        let layouted_line =
                            line_buffer.as_mut_layouted_line(&mut font_system, font_size);

                        for glyph in layouted_line.layouted_glyphs.iter_mut() {
                            glyph.layout_physically(*anchor_point, 1.0);

                            let glyph_color = match glyph.color {
                                Some(color) => color,
                                None => text::color::Color::rgba(0, 0, 0, 255),
                            };

                            rasterizer_cache.with_pixels(
                                &mut font_system,
                                glyph.cache_key.unwrap(),
                                glyph_color,
                                |x, y, color| {
                                    draw_filled_rectangle(
                                        glyph.physical_x_offset.unwrap() + x,
                                        glyph.physical_y_offset.unwrap() + y,
                                        1,
                                        1,
                                        color,
                                    );
                                },
                            );
                        }

                        layouted_line_buffers.push(line_buffer);
                    }
                }
            }

            let calculate_cursor_position = || {
                for (line_index, line_buffer) in layouted_line_buffers.iter().enumerate() {
                    if editing_cursor.line_index == line_index {
                        let layouted_line = line_buffer.layouted_line.as_ref().unwrap();

                        for (glyph_index, glyph) in layouted_line.layouted_glyphs.iter().enumerate()
                        {
                            if editing_cursor.line_index == glyph.start_index {
                                return Some((glyph_index, 0.0));
                            } else if editing_cursor.line_index > glyph.start_index
                                && editing_cursor.line_index < glyph.end_index
                            {
                                // Guess the horizontal offset based on the characters
                                let mut before = 0;
                                let mut total = 0;

                                let cluster = &line_buffer.text[glyph.start_index..glyph.end_index];
                                for (i, _) in cluster.grapheme_indices(true) {
                                    if glyph.start_index + i < editing_cursor.line_index {
                                        before += 1;
                                    }
                                    total += 1;
                                }

                                let offset = glyph.width * (before as f32) / (total as f32);
                                return Some((glyph_index, offset));
                            }
                        }
                        match layouted_line.layouted_glyphs.last() {
                            Some(glyph) => {
                                if editing_cursor.line_index == glyph.end_index {
                                    return Some((layouted_line.layouted_glyphs.len(), 0.0));
                                }
                            }
                            None => {
                                return Some((0, 0.0));
                            }
                        }
                    }
                }

                None
            };

            if let Some((cursor_glyph_index, cursor_glyph_horizontal_offset)) =
                calculate_cursor_position()
            {}

            // Draw the hitboxes of the glyphs after they've been laid out and the line boundaries
            for (line_buffer, document_element) in
                layouted_line_buffers.iter().zip(document_elements.iter())
            {
                let layouted_line = line_buffer.layouted_line.as_ref().unwrap();

                for glyph in layouted_line.layouted_glyphs.iter() {
                    let overlay_rectangle = Rect::from_xywh(
                        glyph.physical_x_offset.unwrap() as f32,
                        glyph.physical_y_offset.unwrap() as f32 - glyph.y_origin * font_size,
                        glyph.width,
                        glyph.height,
                    );

                    let mut glyph_outline_path = Path::new();
                    let (x, y) = (overlay_rectangle.x(), overlay_rectangle.y());
                    glyph_outline_path.move_to((x, y));
                    glyph_outline_path.line_to((x + overlay_rectangle.width(), y));
                    glyph_outline_path.line_to((
                        x + overlay_rectangle.width(),
                        y - overlay_rectangle.height(),
                    ));
                    glyph_outline_path.line_to((x, y - overlay_rectangle.height()));
                    glyph_outline_path.close();

                    let mut painting_options = Paint::default();
                    painting_options.set_color(skia_safe::Color::from_argb(128, 0, 0, 255));
                    painting_options.set_stroke_width(1.0);
                    painting_options.set_stroke(true);

                    canvas.draw_path(&glyph_outline_path, &painting_options);
                }

                match document_element {
                    DocumentElement::Line { anchor_point, .. } => {
                        let mut painting_options = Paint::default();
                        painting_options.set_color(skia_safe::Color::from_argb(128, 255, 0, 0));
                        painting_options.set_stroke_width(1.0);
                        painting_options.set_stroke(true);

                        let x_origin = layouted_line
                            .layouted_glyphs
                            .first()
                            .unwrap()
                            .physical_x_offset
                            .unwrap() as f32;
                        let last_glyph = layouted_line.layouted_glyphs.last().unwrap();
                        let x_reach =
                            last_glyph.physical_x_offset.unwrap() as f32 + last_glyph.width;

                        let mut line_top_path = Path::new();
                        line_top_path
                            .move_to((x_origin, -layouted_line.maximum_y_reach + anchor_point.1));
                        line_top_path
                            .line_to((x_reach, -layouted_line.maximum_y_reach + anchor_point.1));

                        canvas.draw_path(&line_top_path, &painting_options);

                        let mut line_bottom_path = Path::new();
                        line_bottom_path
                            .move_to((x_origin, -layouted_line.minimum_y_origin + anchor_point.1));
                        line_bottom_path
                            .line_to((x_reach, -layouted_line.minimum_y_origin + anchor_point.1));

                        canvas.draw_path(&line_bottom_path, &painting_options);
                    }
                }
            }

            // Copy Skia surface to Cairo context
            let image_snapshot = surface.image_snapshot();
            let image_info = image_snapshot.image_info();
            let mut pixmap = vec![0; (image_info.width() * image_info.height() * 4) as usize];
            image_snapshot.read_pixels(
                image_info,
                pixmap.as_mut_slice(),
                image_info.min_row_bytes(),
                (0, 0),
                CachingHint::Allow,
            );

            let surface = cairo::ImageSurface::create_for_data(
                pixmap,
                cairo::Format::ARgb32,
                image_info.width(),
                image_info.height(),
                image_info.min_row_bytes() as i32,
            )
            .unwrap();

            cairo_context.scale(1.0 / scale_factor as f64, 1.0 / scale_factor as f64);
            cairo_context
                .set_source_surface(&surface, 0.0, 0.0)
                .unwrap();
            cairo_context.paint().unwrap();
        });

        window.set_child(Some(&drawing_area));

        window.present();
    });
    application.run()
}
