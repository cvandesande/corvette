use crate::shape::Shape;

/// Borrowed grey picture, rows stored top to bottom and columns left to right, one byte per pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane<'a> {
    data: &'a [u8],
    shape: Shape,
}

impl<'a> Plane<'a> {
    /// Returns the plane, or `None` unless `data.len()` equals `shape.pixels()`.
    #[must_use]
    pub const fn new(data: &'a [u8], shape: Shape) -> Option<Self> {
        if data.len() == shape.pixels() {
            Some(Self { data, shape })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn data(self) -> &'a [u8] {
        self.data
    }

    #[must_use]
    pub const fn shape(self) -> Shape {
        self.shape
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    data: Vec<u8>,
    shape: Shape,
}

impl Image {
    /// Returns the image, or `None` unless `data.len()` equals `shape.pixels()`.
    #[must_use]
    pub fn new(data: Vec<u8>, shape: Shape) -> Option<Self> {
        if data.len() == shape.pixels() {
            Some(Self { data, shape })
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) fn zeroed(shape: Shape) -> Self {
        Self {
            data: vec![0u8; shape.pixels()],
            shape,
        }
    }

    #[must_use]
    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub const fn shape(&self) -> Shape {
        self.shape
    }

    #[must_use]
    pub fn plane(&self) -> Plane<'_> {
        Plane {
            data: &self.data,
            shape: self.shape,
        }
    }

    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}
