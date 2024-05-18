use std::{num::NonZeroU32, rc::Rc};

use text::{
    attributes::{Attributes, Style, Weight},
    color::Color,
    font_system::FontSystem,
    line_buffer::LineBuffer,
    swash_cache::SwashCache,
};
use tiny_skia::{Paint, PixmapMut, Transform};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use winit::{
    dpi::PhysicalPosition,
    event::{ElementState, Event, MouseButton, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    keyboard::Key,
    window::WindowBuilder,
};

#[derive(Debug, Clone)]
enum UserEvent {
    RequestRedraw,
    InsertText(String),
}

// NOTE(ghovax): This could be serialized, but I need to find a way to serialize `Attrs`.
enum DocumentElement {
    Line {
        anchor_point: PhysicalPosition<f32>,
        spans: Vec<(String, Attributes)>,
    },
}

fn main() {
    // Initialize the logging handler
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Create the window and graphics context to draw to
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event()
        .build()
        .unwrap();
    let event_loop_proxy = event_loop.create_proxy();
    let window = Rc::new(WindowBuilder::new().build(&event_loop).unwrap());
    let context = softbuffer::Context::new(window.clone()).unwrap();
    let mut surface = softbuffer::Surface::new(&context, window.clone()).unwrap();

    // Get the font handling loaded-up
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();

    let mut display_scale = window.scale_factor() as f32;

    // NOTE(ghovax): This parameter could be configured.
    let font_size = 32.0 * display_scale;

    let default_attributes = Attributes::new();
    // NOTE(ghovax): There is definitely a better way to do this.
    let italic_attributes = {
        let mut attributes = Attributes::new();
        attributes.style = Style::Italic;
        attributes
    };
    let bold_attributes = {
        let mut attributes = Attributes::new();
        attributes.weight = Weight::BOLD;
        attributes
    };

    // NOTE(ghovax): This could be loaded from an actual document.
    let document = vec![
        DocumentElement::Line {
            anchor_point: PhysicalPosition::new(85.0, 120.0),
            spans: vec![
                ("B".to_string(), default_attributes),
                ("old ".to_string(), italic_attributes),
                ("example text".to_string(), bold_attributes),
            ],
        },
        DocumentElement::Line {
            anchor_point: PhysicalPosition::new(35.0, 180.0),
            spans: vec![
                ("B".to_string(), default_attributes),
                ("old ".to_string(), italic_attributes),
                ("example text".to_string(), bold_attributes),
            ],
        },
    ];

    // Create all the line buffers from the respective document elements
    // NOTE(ghovax): This is not generalizeable to actual full documents with mixed content.
    let mut line_buffers = Vec::new();
    for document_element in document.iter() {
        match document_element {
            DocumentElement::Line { spans, .. } => {
                let line_buffer = LineBuffer::from_rich_text(spans, default_attributes);
                line_buffers.push(line_buffer);
            }
        }
    }

    // TODO(ghovax): Figure out how to position the cursor at each line.
    let mut mouse_position = PhysicalPosition::new(0.0, 0.0);
    let mut mouse_left_button_state = ElementState::Released;

    event_loop
        .run(|event, event_loop_window_target| {
            event_loop_window_target.set_control_flow(ControlFlow::Wait);

            #[allow(clippy::single_match, clippy::collapsible_match)]
            match event {
                Event::UserEvent(user_event) => match user_event {
                    UserEvent::RequestRedraw => {
                        let (width, height) = {
                            let size = window.inner_size();
                            (size.width, size.height)
                        };

                        surface
                            .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
                            .unwrap();

                        let mut surface_buffer = surface.buffer_mut().unwrap();
                        let surface_buffer_data = unsafe {
                            std::slice::from_raw_parts_mut(
                                surface_buffer.as_mut_ptr() as *mut u8,
                                surface_buffer.len() * 4,
                            )
                        };
                        let mut surface_pixel_map = PixmapMut::from_bytes(surface_buffer_data, width, height).unwrap();
                        surface_pixel_map.fill(tiny_skia::Color::WHITE);


                        let mut painting_options = Paint::default();
                        let mut paint_rectangle = |x, y, width, height, color: Color| {
                            // NOTE(ghovax): Due to `softbuffer`` and `tiny_skia` having incompatible internal color
                            // representations we swap the red and blue channels here
                            painting_options.set_color_rgba8(color.b(), color.g(), color.r(), color.a());
                            surface_pixel_map.fill_rect(
                                tiny_skia::Rect::from_xywh(x as f32, y as f32, width as f32, height as f32).unwrap(),
                                &painting_options,
                                Transform::identity(),
                                None,
                            );
                        };

                        for (line_buffer, document_element) in line_buffers.iter_mut().zip(document.iter()) {
                            let anchor_point = match document_element {
                                DocumentElement::Line { anchor_point, .. } => *anchor_point,
                            };
                            let layouted_line = line_buffer.as_layouted_line(&mut font_system, font_size);

                            for glyph in layouted_line.layouted_glyphs.iter() {
                                let physical_glyph = glyph.layout_physically((anchor_point.x, anchor_point.y), 1.0);

                                let glyph_color = match glyph.color {
                                    Some(color) => color,
                                    None => Color::rgba(0, 0, 0, 255),
                                };

                                swash_cache.with_pixels(
                                    &mut font_system,
                                    physical_glyph.cache_key,
                                    glyph_color,
                                    |x, y, color| {
                                        paint_rectangle(
                                            physical_glyph.x_offset + x,
                                            physical_glyph.y_offset + y,
                                            1,
                                            1,
                                            color,
                                        );
                                    },
                                );
                            }
                        }

                        surface_buffer.present().unwrap();
                    }
                    UserEvent::InsertText(text) => {
                        for character in text.chars() {
                            if character.is_control() && !['\t', '\n', '\r', '\u{92}'].contains(&character) {
                                // Filter out special chars (except for tab)
                                log::debug!("Refusing to insert control character {:?}", character);
                            } else if ['\n', '\r'].contains(&character) {
                                log::debug!("Received enter input, still have to implement the functionality");
                            } else {
                                // TODO
                            }
                        }
                    }
                },
                Event::WindowEvent { window_id, event } => match event {
                    WindowEvent::CloseRequested => {
                        event_loop_window_target.exit();
                    }
                    WindowEvent::KeyboardInput {
                        event: winit::event::KeyEvent { logical_key, state, .. },
                        ..
                    } => {
                        if state.is_pressed() {
                            match logical_key {
                                Key::Named(key) => {
                                    if let Some(text) = key.to_text() {
                                        event_loop_proxy
                                            .send_event(UserEvent::InsertText(text.to_string()))
                                            .unwrap();
                                        event_loop_proxy.send_event(UserEvent::RequestRedraw).unwrap();
                                    }
                                }
                                Key::Character(text) => {
                                    event_loop_proxy
                                        .send_event(UserEvent::InsertText(text.to_string()))
                                        .unwrap();
                                    event_loop_proxy.send_event(UserEvent::RequestRedraw).unwrap();
                                }
                                _ => {}
                            }
                        }
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        log::info!("Updated scale factor for the window with ID {window_id:?}");

                        display_scale = scale_factor as f32;
                        for line_buffer in line_buffers.iter_mut() {
                            // TODO(ghovax): I need to change the metrics of the font.
                        }

                        event_loop_proxy.send_event(UserEvent::RequestRedraw).unwrap();
                    }
                    WindowEvent::Resized(size) => {
                        event_loop_proxy.send_event(UserEvent::RequestRedraw).unwrap();
                    }
                    WindowEvent::CursorMoved { device_id: _, position } => {
                        mouse_position = position;
                    }
                    WindowEvent::MouseInput {
                        device_id: _,
                        state,
                        button,
                    } => {
                        if button == MouseButton::Left {
                            if state == ElementState::Pressed && mouse_left_button_state == ElementState::Released {
                                for ((line_index, line_buffer), document_element) in
                                    line_buffers.iter().enumerate().zip(document.iter())
                                {
                                    let anchor_point = match document_element {
                                        DocumentElement::Line { anchor_point, .. } => anchor_point,
                                    };
                                    // TODO(ghovax): Position the cursor.
                                }
                            }

                            mouse_left_button_state = state;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        })
        .unwrap();
}
