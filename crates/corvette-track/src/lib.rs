//! Pure tracking logic for the Corvette detector: the object filters, the box
//! mapping and the detection merge.
pub mod distance;
pub mod filters;
pub mod kalman;
pub mod mapping;
pub mod matcher;
pub mod merge;
pub mod object_tracker;
pub mod reduce;
pub mod score;
pub mod stationary;
pub mod track;
pub mod tracker;
pub use distance::{Matrix, distance, distance_matrix};
pub use filters::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};
pub use kalman::Kalman;
pub use mapping::{Frame, Geometry, Region, Skipped, map_box};
pub use matcher::{Matching, NanDistance, Pair, match_greedy};
pub use merge::{Detection, clipped, consolidate, inclusive_area, intersection};
pub use object_tracker::{FrameDetection, ObjectTracker, TrackerSettings};
pub use reduce::reduce_detections;
pub use score::Scores;
pub use stationary::{Stationary, StationaryThresholds, Verdict};
pub use track::{IdSource, Observation, Track, TrackConfig};
pub use tracker::{Sighting, Tracker};
