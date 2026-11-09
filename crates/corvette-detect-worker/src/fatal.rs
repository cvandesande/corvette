pub const EXIT_DEVICE_LOST: i32 = 20;
pub const EXIT_DEADLINE_OVERRUN: i32 = 21;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatalReason {
    DeviceLost,
    DeadlineOverrun,
}

impl FatalReason {
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::DeviceLost => EXIT_DEVICE_LOST,
            Self::DeadlineOverrun => EXIT_DEADLINE_OVERRUN,
        }
    }
}

/// The injected process exit, so a test can record instead of ending the process.
pub trait Terminator: Sync {
    fn terminate(&self, code: i32);
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessExit;

impl Terminator for ProcessExit {
    fn terminate(&self, code: i32) {
        std::process::exit(code);
    }
}

pub fn fatal(terminator: &dyn Terminator, reason: FatalReason, detail: &str) {
    eprintln!("corvette-detect-worker: fatal: {reason:?}: {detail}");
    terminator.terminate(reason.exit_code());
}
