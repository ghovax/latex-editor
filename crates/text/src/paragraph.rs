use crate::line_buffer::LineBuffer;

/// Align or justify.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Alignment {
    Left,
    Right,
    Center,
    Justified,
}

pub struct Paragraph {
    pub lines: Vec<LineBuffer>,
    pub line_heights: Vec<f32>,
    pub alignment: Alignment,
}
