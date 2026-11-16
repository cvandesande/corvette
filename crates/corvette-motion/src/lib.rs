//! Pure image primitives for the Corvette motion detector.
#![forbid(unsafe_code)]

pub mod average;
pub mod blur;
pub mod colour;
pub mod contours;
pub mod contrast;
pub mod contrast_history;
pub mod crop;
pub mod decision;
pub mod detector;
pub mod difference;
pub mod dilate;
pub mod mask;
pub mod plane;
pub mod polygon;
pub mod resize;
pub mod resize_rgb;
pub mod shape;
pub mod shrink;

pub use average::RunningAverage;
pub use blur::{GAUSSIAN_CENTRE_WEIGHT, GAUSSIAN_OUTER_WEIGHT, gaussian_blur};
pub use colour::i420_to_rgb;
pub use contours::{Contour, external_contours};
pub use contrast::contrast_bounds;
pub use contrast_history::ContrastHistory;
pub use crop::{Region, RegionError, crop_pad_i420};
pub use decision::{Decision, MotionBox, MotionDecider, MotionSettings};
pub use detector::MotionDetector;
pub use difference::{absolute_difference, threshold_above};
pub use dilate::dilate_3x3;
pub use mask::motion_keep_mask;
pub use plane::{Image, Plane};
pub use polygon::fill_polygons;
pub use resize::resize_nearest;
pub use resize_rgb::resize_linear_rgb;
pub use shape::{Shape, motion_shape};
pub use shrink::shrink_keep_mask;
