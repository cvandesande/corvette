//! Keep-alive interval derivation and method fallback.
//!
//! See `docs/design/api-contracts.md`'s "RTSP camera sessions require a real
//! keep-alive margin, not a thin one" section: a camera silently stops
//! sending RTP frames at its own declared `SETUP` timeout, with no TCP close
//! and no RTSP error. Corvette's contract is that the scheduled interval is
//! always computed from that declared value, never a fixed constant.

use rtsp_types::{Method, StatusCode};
use std::time::Duration;

/// Applied when a camera's `SETUP` response omits `timeout=` entirely.
///
/// No camera this project has tested has ever omitted it (both directly
/// tested Reolink cameras declared 65 seconds), so there is no empirical
/// value to copy here. RFC 2326 §12.37 states the same default for exactly
/// this situation ("the default is 60 seconds"), which this client adopts
/// unchanged rather than inventing its own number.
pub(super) const DEFAULT_DECLARED_TIMEOUT: Duration = Duration::from_mins(1);

/// The fraction of the declared timeout the keep-alive interval targets.
///
/// `docs/design/api-contracts.md` records that sending a keep-alive every 20
/// seconds against a 65-second declared timeout (roughly a third) keeps a
/// real camera's stream flowing indefinitely with zero gaps. This constant
/// is that ratio, applied to whatever timeout a given camera declares --
/// not a fixed number of seconds.
const MARGIN_DIVISOR: u32 = 3;

/// The loosest margin INV-3 tolerates: the interval must never be closer
/// than half the declared timeout, regardless of `MARGIN_DIVISOR`. Guards
/// the invariant even if `MARGIN_DIVISOR` is loosened by a future edit.
const MAX_INTERVAL_DIVISOR: u32 = 2;

/// Computes the keep-alive send interval for a camera's declared `SETUP`
/// timeout (INV-3): a real fraction of the declared value, capped so it
/// never exceeds half of it even for a very short declared timeout.
pub(super) fn keep_alive_interval(declared_timeout: Duration) -> Duration {
    let by_ratio = declared_timeout / MARGIN_DIVISOR;
    let half = declared_timeout / MAX_INTERVAL_DIVISOR;
    by_ratio.min(half)
}

/// Which method the scheduler currently sends for keep-alives. Starts at
/// `GetParameter`; falls back to `Options` if a camera ever rejects
/// `GET_PARAMETER` outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeepAliveMethod {
    GetParameter,
    Options,
}

impl KeepAliveMethod {
    pub(super) const fn as_rtsp_method(self) -> Method {
        match self {
            Self::GetParameter => Method::GetParameter,
            Self::Options => Method::Options,
        }
    }
}

/// Tracks the current keep-alive method for one camera session, falling
/// back from `GET_PARAMETER` to `OPTIONS` the first time a camera rejects
/// `GET_PARAMETER` as an unsupported or invalid method.
#[derive(Debug, Clone, Copy)]
pub(super) struct KeepAliveScheduler {
    interval: Duration,
    method: KeepAliveMethod,
}

impl KeepAliveScheduler {
    pub(super) fn new(declared_timeout: Duration) -> Self {
        Self {
            interval: keep_alive_interval(declared_timeout),
            method: KeepAliveMethod::GetParameter,
        }
    }

    pub(super) const fn interval(&self) -> Duration {
        self.interval
    }

    pub(super) const fn method(&self) -> KeepAliveMethod {
        self.method
    }

    /// Updates fallback state from the response to a keep-alive request
    /// sent with `sent_method`. Falls back to `OPTIONS` on the first
    /// rejection of `GET_PARAMETER`; `OPTIONS` itself has no further
    /// fallback (RFC 2326 requires every RTSP server to implement it).
    pub(super) fn observe_response(&mut self, sent_method: KeepAliveMethod, status: StatusCode) {
        let rejected = matches!(
            status,
            StatusCode::MethodNotAllowed
                | StatusCode::NotImplemented
                | StatusCode::ParameterNotUnderstood
        );
        if sent_method == KeepAliveMethod::GetParameter && rejected {
            self.method = KeepAliveMethod::Options;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_is_a_real_margin_under_several_declared_timeouts() {
        // INV-3: strictly less than declared, never closer than half.
        for declared_secs in [30_u64, 65, 120] {
            let declared = Duration::from_secs(declared_secs);
            let interval = keep_alive_interval(declared);
            assert!(
                interval < declared,
                "interval {interval:?} must be strictly less than declared {declared:?}"
            );
            assert!(
                interval <= declared / 2,
                "interval {interval:?} must be no closer than half of declared {declared:?}"
            );
        }
    }

    #[test]
    fn interval_scales_with_the_declared_value_rather_than_a_constant() {
        let short = keep_alive_interval(Duration::from_secs(30));
        let long = keep_alive_interval(Duration::from_mins(2));
        assert!(
            long > short,
            "a longer declared timeout must produce a longer interval, not a fixed one"
        );
    }

    #[test]
    fn falls_back_to_options_after_get_parameter_is_rejected() {
        let mut scheduler = KeepAliveScheduler::new(Duration::from_secs(65));
        assert_eq!(scheduler.method(), KeepAliveMethod::GetParameter);

        scheduler.observe_response(KeepAliveMethod::GetParameter, StatusCode::NotImplemented);
        assert_eq!(scheduler.method(), KeepAliveMethod::Options);
    }

    #[test]
    fn does_not_fall_back_on_get_parameter_success() {
        let mut scheduler = KeepAliveScheduler::new(Duration::from_secs(65));
        scheduler.observe_response(KeepAliveMethod::GetParameter, StatusCode::Ok);
        assert_eq!(scheduler.method(), KeepAliveMethod::GetParameter);
    }
}
