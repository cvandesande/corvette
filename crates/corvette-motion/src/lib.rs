//! Pure image primitives for the Corvette motion detector.
#![forbid(unsafe_code)]

pub mod plane;
pub mod shape;

pub use plane::{Image, Plane};
pub use shape::{Shape, motion_shape};
