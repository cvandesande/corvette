//! Pure tracking logic for the Corvette detector: the object filters, the box
//! mapping and the detection merge.
pub mod filters;
pub mod kalman;
pub mod mapping;
pub mod merge;
pub mod reduce;
pub use filters::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};
pub use kalman::Kalman;
pub use mapping::{Frame, Geometry, Region, Skipped, map_box};
pub use merge::{Detection, clipped, consolidate, inclusive_area, intersection};
pub use reduce::reduce_detections;
