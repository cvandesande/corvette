//! Pure image primitives for the Corvette motion detector.
#![forbid(unsafe_code)]

pub mod blur;
pub mod plane;
pub mod resize;
pub mod shape;

pub use blur::{GAUSSIAN_CENTRE_WEIGHT, GAUSSIAN_OUTER_WEIGHT, gaussian_blur};
pub use plane::{Image, Plane};
pub use resize::resize_nearest;
pub use shape::{Shape, motion_shape};
