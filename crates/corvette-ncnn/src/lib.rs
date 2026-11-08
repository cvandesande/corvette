//! The safe layer over `ncnn-sys` for the Corvette detection worker.

pub mod device;
pub mod error;
pub mod handle;
pub mod net;

use ncnn_sys::{ncnn_ext_destroy_gpu_instance, ncnn_version};

pub use ncnn_sys::DeviceType;

pub use device::{Device, DeviceInfo};
pub use error::{Error, LoadStage};
pub use handle::LiveHandles;
pub use net::{Fp16Snapshot, InputShape, Net, OptionSnapshot, Output};

#[must_use]
pub fn version() -> String {
    // SAFETY: ncnn_version takes no arguments and returns a static
    // NUL-terminated string that stays valid for the whole process.
    let raw = unsafe { ncnn_version() };
    handle::cstr_to_string(raw).unwrap_or_default()
}

#[must_use]
pub fn live_handles() -> LiveHandles {
    handle::live_handles()
}

/// # Errors
///
/// [`Error::NetsAlive`] when a [`Net`] is still alive; the instance is not destroyed then.
pub fn release_gpu_instance() -> Result<(), Error> {
    let _guard = handle::lock_instance();
    let alive = handle::live_nets();
    if alive != 0 {
        return Err(Error::NetsAlive { count: alive });
    }
    // SAFETY: nets are created and destroyed only under this lock and stay
    // counted across their destroy, so a zero count here outlives the call.
    unsafe { ncnn_ext_destroy_gpu_instance() };
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::handle::lock_instance;
    use crate::{Device, Error, InputShape, LoadStage, Net};

    const BLOCKED: Duration = Duration::from_millis(300);
    const UNBLOCKED: Duration = Duration::from_secs(20);

    fn result_after_unlock<T, F>(work: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let guard = lock_instance();
        let (tx, rx) = mpsc::channel();
        let spawned = thread::spawn(move || tx.send(work()));
        assert!(
            matches!(
                rx.recv_timeout(BLOCKED),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "the call finished while the instance lock was still held"
        );
        drop(guard);
        let result = rx
            .recv_timeout(UNBLOCKED)
            .expect("the call produced no result after the unlock");
        assert!(spawned.join().is_ok(), "the spawned thread panicked");
        result
    }

    #[test]
    fn release_waits_for_the_instance_lock() {
        assert!(matches!(
            result_after_unlock(crate::release_gpu_instance),
            Ok(())
        ));
    }

    #[test]
    fn net_load_waits_for_the_instance_lock() {
        let result = result_after_unlock(|| {
            Net::load(
                std::path::Path::new("/nonexistent/lock-test.param"),
                std::path::Path::new("/nonexistent/lock-test.bin"),
                Device::Cpu,
                InputShape {
                    width: 4,
                    height: 4,
                    channels: 3,
                },
            )
            .map(drop)
        });
        assert!(matches!(
            result,
            Err(Error::Load {
                stage: LoadStage::Param,
                ..
            })
        ));
    }

    #[test]
    fn enumerate_waits_for_the_instance_lock() {
        let _devices = result_after_unlock(Device::enumerate);
    }
}
