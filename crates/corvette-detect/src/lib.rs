//! Starts one operating-system child process, watches it, restarts it with a doubling delay, and kills it when the guard is dropped.
#![forbid(unsafe_code)]

mod backoff;
mod supervised;

pub use backoff::{BACKOFF_INITIAL, BACKOFF_MAX, Backoff, DEFAULT_POLICY, HEALTHY_RUN, Policy};
pub use supervised::{Status, Supervised};

pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-detect unit=\"{unit}\" event={event} detail=\"{detail}\"");
}
