use ncnn_sys::{
    DeviceType, ncnn_ext_get_default_gpu_index, ncnn_ext_get_gpu_count,
    ncnn_ext_get_gpu_device_name, ncnn_ext_get_gpu_driver_name, ncnn_ext_get_gpu_rough_score,
    ncnn_ext_get_gpu_type,
};

use crate::error::Error;
use crate::handle::{cstr_to_string, lock_instance};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cpu,
    Vulkan {
        /// The ncnn device index.
        index: i32,
    },
}

#[derive(Debug)]
pub struct DeviceInfo {
    pub index: i32,
    pub name: String,
    pub driver: String,
    pub kind: DeviceType,
    pub score: u32,
}

impl Device {
    #[must_use]
    pub fn enumerate() -> Vec<DeviceInfo> {
        let _lock = lock_instance();
        enumerate_unlocked()
    }

    /// # Errors
    /// [`Error::NoDevice`] when nothing enumerated, [`Error::DeviceOutOfRange`] for an index outside
    /// the list, [`Error::SoftwareDevice`] for a `DeviceType::Cpu` pick when `allow_software` is false.
    pub fn select(requested: Option<i32>, allow_software: bool) -> Result<Self, Error> {
        let _lock = lock_instance();
        let devices = enumerate_unlocked();
        // SAFETY: takes no arguments; -1 means "no default", which `choose`
        // rejects as out of range.
        let default_index = unsafe { ncnn_ext_get_default_gpu_index() };
        choose(&devices, default_index, requested, allow_software)
            .map(|index| Self::Vulkan { index })
    }
}

/// Enumerates every device ncnn can see; call with the instance lock held.
fn enumerate_unlocked() -> Vec<DeviceInfo> {
    // SAFETY: ncnn_ext_get_gpu_count takes no arguments; the shim creates
    // the Vulkan instance on first call rather than assuming one exists.
    let count = unsafe { ncnn_ext_get_gpu_count() };
    if count <= 0 {
        return Vec::new();
    }
    let mut devices = Vec::new();
    for index in 0..count {
        // SAFETY: the shim bounds-checks each index (this loop stays inside
        // `count`) and returns -1 or null for a miss, mapped below.
        let (kind, name, driver, score) = unsafe {
            (
                DeviceType::from_raw(ncnn_ext_get_gpu_type(index)),
                cstr_to_string(ncnn_ext_get_gpu_device_name(index)).unwrap_or_default(),
                cstr_to_string(ncnn_ext_get_gpu_driver_name(index)).unwrap_or_default(),
                ncnn_ext_get_gpu_rough_score(index),
            )
        };
        devices.push(DeviceInfo {
            index,
            name,
            driver,
            kind,
            score,
        });
    }
    devices
}

fn choose(
    devices: &[DeviceInfo],
    default_index: i32,
    requested: Option<i32>,
    allow_software: bool,
) -> Result<i32, Error> {
    if devices.is_empty() {
        return Err(Error::NoDevice);
    }
    let index = requested.unwrap_or(default_index);
    let count = devices.len();
    let position = usize::try_from(index)
        .ok()
        .and_then(|position| devices.get(position).map(|_| position))
        .ok_or(Error::DeviceOutOfRange { index, count })?;
    let device = &devices[position];
    if device.kind == DeviceType::Cpu && !allow_software {
        return Err(Error::SoftwareDevice {
            index,
            name: device.name.clone(),
        });
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::{DeviceInfo, choose};
    use crate::error::Error;
    use ncnn_sys::DeviceType;

    fn device(index: i32, kind: DeviceType) -> DeviceInfo {
        DeviceInfo {
            index,
            name: format!("device-{index}"),
            driver: "test-driver".to_string(),
            kind,
            score: 10,
        }
    }

    #[test]
    fn no_device_enumerated_is_refused() {
        assert!(matches!(choose(&[], -1, None, false), Err(Error::NoDevice)));
    }

    #[test]
    fn index_out_of_range_is_refused() {
        let devices = [device(0, DeviceType::Discrete)];
        assert!(matches!(
            choose(&devices, 0, Some(3), false),
            Err(Error::DeviceOutOfRange { index: 3, count: 1 })
        ));
        assert!(matches!(
            choose(&devices, 0, Some(-1), false),
            Err(Error::DeviceOutOfRange {
                index: -1,
                count: 1
            })
        ));
    }

    #[test]
    fn index_at_the_device_count_boundary_is_refused() {
        let one = [device(0, DeviceType::Discrete)];
        assert!(matches!(
            choose(&one, 0, Some(1), false),
            Err(Error::DeviceOutOfRange { index: 1, count: 1 })
        ));
        let two = [
            device(0, DeviceType::Discrete),
            device(1, DeviceType::Discrete),
        ];
        assert!(matches!(
            choose(&two, 0, Some(2), false),
            Err(Error::DeviceOutOfRange { index: 2, count: 2 })
        ));
    }

    #[test]
    fn software_device_is_refused() {
        let devices = [device(0, DeviceType::Discrete), device(1, DeviceType::Cpu)];
        assert!(matches!(
            choose(&devices, 0, Some(1), false),
            Err(Error::SoftwareDevice { index: 1, name }) if name == "device-1"
        ));
    }

    #[test]
    fn software_device_is_allowed_on_request() {
        let devices = [device(0, DeviceType::Discrete), device(1, DeviceType::Cpu)];
        assert!(matches!(choose(&devices, 0, Some(1), true), Ok(1)));
    }

    #[test]
    fn default_index_is_used_when_none_is_requested() {
        let devices = [
            device(0, DeviceType::Discrete),
            device(1, DeviceType::Discrete),
        ];
        assert!(matches!(choose(&devices, 1, None, false), Ok(1)));
    }

    #[test]
    fn requested_index_wins_over_the_default() {
        let devices = [
            device(0, DeviceType::Discrete),
            device(1, DeviceType::Discrete),
        ];
        assert!(matches!(choose(&devices, 0, Some(1), false), Ok(1)));
    }
}
