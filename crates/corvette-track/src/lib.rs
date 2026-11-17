//! Pure tracking logic for the Corvette detector: the object filters, the box
//! mapping and the detection merge.
pub mod distance;
pub mod filters;
pub mod kalman;
pub mod mapping;
pub mod matcher;
pub mod merge;
pub mod reduce;
pub use distance::{Matrix, distance, distance_matrix};
pub use filters::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};
pub use kalman::Kalman;
pub use mapping::{Frame, Geometry, Region, Skipped, map_box};
pub use matcher::{Matching, NanDistance, Pair, match_greedy};
pub use merge::{Detection, clipped, consolidate, inclusive_area, intersection};
pub use reduce::reduce_detections;
