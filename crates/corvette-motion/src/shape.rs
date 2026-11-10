/// Width and height of a grey picture, both non-zero, with a pixel count that fits in `usize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    width: usize,
    height: usize,
}

impl Shape {
    /// Returns the shape, or `None` when a side is zero or `width * height` overflows `usize`.
    #[must_use]
    pub const fn new(width: usize, height: usize) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        match width.checked_mul(height) {
            Some(_) => Some(Self { width, height }),
            None => None,
        }
    }

    #[must_use]
    pub const fn width(self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }

    #[must_use]
    pub const fn pixels(self) -> usize {
        self.width * self.height
    }
}

/// Returns `None` for a zero `frame_height`, a product overflow, or a resulting width of 0.
#[must_use]
pub fn motion_shape(frame: Shape, frame_height: usize) -> Option<Shape> {
    let product = frame_height.checked_mul(frame.width())?;
    Shape::new(product / frame.height(), frame_height)
}
