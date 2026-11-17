//! Pure tracking logic for the Corvette detector: the object filters, the box
//! mapping and the detection merge.
pub mod filters;
pub mod mapping;
pub mod merge;
pub use filters::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};
pub use mapping::{Frame, Geometry, Region, Skipped, map_box};
pub use merge::{Detection, clipped, consolidate, inclusive_area, intersection};
