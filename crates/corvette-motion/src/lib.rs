//! Pure image primitives for the Corvette motion detector.
#![forbid(unsafe_code)]

pub mod blur;
pub mod contrast;
pub mod contrast_history;
pub mod difference;
pub mod dilate;
pub mod plane;
pub mod resize;
pub mod shape;

pub use blur::{GAUSSIAN_CENTRE_WEIGHT, GAUSSIAN_OUTER_WEIGHT, gaussian_blur};
pub use contrast::contrast_bounds;
pub use contrast_history::ContrastHistory;
pub use difference::{absolute_difference, threshold_above};
pub use dilate::dilate_3x3;
pub use plane::{Image, Plane};
pub use resize::resize_nearest;
pub use shape::{Shape, motion_shape};
