//! Pure tracking logic for the Corvette detector: the object filters.
pub mod filters;
pub mod mapping;
pub use filters::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};
pub use mapping::{Frame, Geometry, Region, Skipped, map_box};
