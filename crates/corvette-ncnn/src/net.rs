use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::path::Path;
use std::ptr;

use ncnn_sys::{
    ncnn_extractor_create, ncnn_extractor_extract, ncnn_extractor_input,
    ncnn_mat_create_external_3d, ncnn_mat_get_c, ncnn_mat_get_cstep, ncnn_mat_get_data,
    ncnn_mat_get_dims, ncnn_mat_get_elempack, ncnn_mat_get_elemsize, ncnn_mat_get_h,
    ncnn_mat_get_w, ncnn_mat_t, ncnn_net_create, ncnn_net_get_input_count, ncnn_net_get_input_name,
    ncnn_net_get_option, ncnn_net_get_output_count, ncnn_net_get_output_name, ncnn_net_load_model,
    ncnn_net_load_param, ncnn_net_set_vulkan_device, ncnn_net_t,
    ncnn_option_get_use_fp16_arithmetic, ncnn_option_get_use_fp16_packed,
    ncnn_option_get_use_fp16_storage, ncnn_option_get_use_vulkan_compute,
};

use crate::device::Device;
use crate::error::{Error, LoadStage};
use crate::handle::{ExtractorGuard, MatGuard, NetGuard, cstr_to_string, lock_instance};

/// The single value all three fp16 options are set from: 0, so fp16 is off.
const FP16: c_int = 0;

/// A shape is accepted only when `width * height` is a multiple of 4: ncnn
/// spaces the channels of an external Mat by the plane size rounded up to 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputShape {
    pub width: i32,
    pub height: i32,
    pub channels: i32,
}

impl InputShape {
    fn element_count(self) -> Result<usize, Error> {
        let side = |value: i32| usize::try_from(value).ok().filter(|n| *n > 0);
        match (side(self.width), side(self.height), side(self.channels)) {
            (Some(w), Some(h), Some(c)) => w
                .checked_mul(h)
                .filter(|plane| plane % 4 == 0)
                .and_then(|plane| plane.checked_mul(c))
                .ok_or(Error::InvalidInputShape),
            _ => Err(Error::InvalidInputShape),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionSnapshot {
    pub vulkan_compute: bool,
    pub fp16: Fp16Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fp16Snapshot {
    pub packed: bool,
    pub storage: bool,
    pub arithmetic: bool,
}

#[derive(Debug)]
pub struct Output {
    pub shape: Vec<i32>,
    pub data: Vec<f32>,
}

#[derive(Debug)]
pub struct Net {
    guard: NetGuard,
    device: Device,
    shape: InputShape,
    expected: usize,
    input_name: String,
    output_name: String,
    input_blob: CString,
    output_blob: CString,
}

impl Net {
    /// # Errors
    /// `InvalidInputShape`, `InvalidPath`, `NullHandle`, `VulkanNotEnabled`, `Load`, `NoInputBlob` or `NoOutputBlob`.
    pub fn load(
        param: &Path,
        bin: &Path,
        device: Device,
        input: InputShape,
    ) -> Result<Self, Error> {
        let expected = input.element_count()?;
        let param_blob = path_to_cstring(param)?;
        let bin_blob = path_to_cstring(bin)?;
        let _lock = lock_instance();

        // SAFETY: the option pointer borrows the net the guard owns from
        // creation; the guard destroys it exactly once, under the lock.
        unsafe {
            let raw = ncnn_net_create();
            if raw.is_null() {
                return Err(Error::NullHandle {
                    what: "ncnn_net_create",
                });
            }
            let net = NetGuard::new(raw);
            configure_options(raw, device)?;
            let code = ncnn_net_load_param(raw, param_blob.as_ptr());
            if code != 0 {
                return Err(Error::Load {
                    stage: LoadStage::Param,
                    path: param.to_path_buf(),
                    code,
                });
            }
            let code = ncnn_net_load_model(raw, bin_blob.as_ptr());
            if code != 0 {
                return Err(Error::Load {
                    stage: LoadStage::Model,
                    path: bin.to_path_buf(),
                    code,
                });
            }
            if ncnn_net_get_input_count(raw) < 1 {
                return Err(Error::NoInputBlob);
            }
            let input_name =
                cstr_to_string(ncnn_net_get_input_name(raw, 0)).ok_or(Error::NoInputBlob)?;
            if ncnn_net_get_output_count(raw) < 1 {
                return Err(Error::NoOutputBlob);
            }
            let output_name =
                cstr_to_string(ncnn_net_get_output_name(raw, 0)).ok_or(Error::NoOutputBlob)?;
            Ok(Self {
                guard: net,
                device,
                shape: input,
                expected,
                input_blob: CString::new(input_name.as_str()).map_err(|_| Error::BlobName)?,
                input_name,
                output_blob: CString::new(output_name.as_str()).map_err(|_| Error::BlobName)?,
                output_name,
            })
        }
    }

    /// # Errors
    /// [`Error::BlobName`] when either name contains an interior NUL byte.
    pub fn with_blobs(mut self, input: &str, output: &str) -> Result<Self, Error> {
        self.input_blob = CString::new(input).map_err(|_| Error::BlobName)?;
        self.output_blob = CString::new(output).map_err(|_| Error::BlobName)?;
        self.input_name = input.to_string();
        self.output_name = output.to_string();
        Ok(self)
    }

    #[must_use]
    pub fn input_name(&self) -> &str {
        &self.input_name
    }

    #[must_use]
    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    #[must_use]
    pub const fn device(&self) -> Device {
        self.device
    }

    #[must_use]
    pub fn options(&self) -> OptionSnapshot {
        // SAFETY: the option pointer borrows this live net; `configure_options`
        // proved it non-null at load, and the getters are pure reads.
        unsafe {
            let opt = ncnn_net_get_option(self.guard.handle());
            OptionSnapshot {
                vulkan_compute: ncnn_option_get_use_vulkan_compute(opt) != 0,
                fp16: Fp16Snapshot {
                    packed: ncnn_option_get_use_fp16_packed(opt) != 0,
                    storage: ncnn_option_get_use_fp16_storage(opt) != 0,
                    arithmetic: ncnn_option_get_use_fp16_arithmetic(opt) != 0,
                },
            }
        }
    }

    /// # Errors
    /// `InputLength` (checked before any ncnn call), `NullHandle`, `SetInput`, `Extract` or `MalformedOutput`.
    pub fn infer(&self, input: &mut [f32]) -> Result<Output, Error> {
        if input.len() != self.expected {
            return Err(Error::InputLength {
                expected: self.expected,
                actual: input.len(),
            });
        }
        // SAFETY: the guards' lifetimes are the borrows ncnn needs — input
        // buffer, net, extractor — and each destroys its handle on every path.
        unsafe {
            let raw_mat = ncnn_mat_create_external_3d(
                self.shape.width,
                self.shape.height,
                self.shape.channels,
                input.as_mut_ptr().cast::<c_void>(),
                ptr::null_mut(),
            );
            if raw_mat.is_null() {
                return Err(Error::NullHandle {
                    what: "ncnn_mat_create_external_3d",
                });
            }
            let input_mat = MatGuard::external(raw_mat, &mut *input);
            let raw_extractor = ncnn_extractor_create(self.guard.handle());
            if raw_extractor.is_null() {
                return Err(Error::NullHandle {
                    what: "ncnn_extractor_create",
                });
            }
            let extractor = ExtractorGuard::new(raw_extractor, &self.guard);
            let code = ncnn_extractor_input(
                extractor.handle(),
                self.input_blob.as_ptr(),
                input_mat.handle(),
            );
            if code != 0 {
                return Err(Error::SetInput {
                    blob: self.input_name.clone(),
                    code,
                });
            }
            let mut raw_output: ncnn_mat_t = ptr::null_mut();
            let code = ncnn_extractor_extract(
                extractor.handle(),
                self.output_blob.as_ptr(),
                &raw mut raw_output,
            );
            let output =
                (!raw_output.is_null()).then(|| MatGuard::extracted(raw_output, &extractor));
            if code != 0 {
                return Err(Error::Extract {
                    blob: self.output_name.clone(),
                    code,
                });
            }
            copy_out(
                output
                    .ok_or(Error::NullHandle {
                        what: "extracted output Mat",
                    })?
                    .handle(),
            )
        }
    }
}

fn configure_options(net: ncnn_net_t, device: Device) -> Result<(), Error> {
    // SAFETY: `net` is a live created handle; the returned option pointer
    // borrows it and stays valid while the net lives.
    let opt = unsafe { ncnn_net_get_option(net) };
    if opt.is_null() {
        return Err(Error::NullHandle {
            what: "ncnn_net_get_option",
        });
    }
    // SAFETY: `opt` borrows the live net checked above; each setter is a
    // plain option write on that borrowed struct, as ncnn's C API requires.
    unsafe {
        ncnn_sys::ncnn_option_set_use_fp16_packed(opt, FP16);
        ncnn_sys::ncnn_option_set_use_fp16_storage(opt, FP16);
        ncnn_sys::ncnn_option_set_use_fp16_arithmetic(opt, FP16);
        let vulkan = i32::from(matches!(device, Device::Vulkan { .. }));
        ncnn_sys::ncnn_option_set_use_vulkan_compute(opt, vulkan);
    }
    if let Device::Vulkan { index } = device {
        // SAFETY: `net` is the live handle this function was given.
        unsafe { ncnn_net_set_vulkan_device(net, index) };
        // SAFETY: `opt` still borrows the live net.
        let stuck = unsafe { ncnn_option_get_use_vulkan_compute(opt) } == 1;
        if !stuck {
            return Err(Error::VulkanNotEnabled);
        }
    }
    Ok(())
}

fn copy_out(mat: ncnn_mat_t) -> Result<Output, Error> {
    let malformed = |reason: String| Error::MalformedOutput { reason };
    // SAFETY: `mat` is a live Mat returned by ncnn_extractor_extract; these
    // are all pure accessors on it.
    let (elemsize, elempack, dims, w, h, c, cstep, data) = unsafe {
        (
            ncnn_mat_get_elemsize(mat),
            ncnn_mat_get_elempack(mat),
            ncnn_mat_get_dims(mat),
            ncnn_mat_get_w(mat),
            ncnn_mat_get_h(mat),
            ncnn_mat_get_c(mat),
            ncnn_mat_get_cstep(mat),
            ncnn_mat_get_data(mat),
        )
    };
    if elemsize != 4 || elempack != 1 {
        return Err(malformed(format!(
            "expected unpacked fp32 output, got elemsize={elemsize} elempack={elempack}"
        )));
    }
    if data.is_null() {
        return Err(malformed("output Mat has no data".to_string()));
    }

    let width =
        usize::try_from(w).map_err(|_| malformed(format!("output width is negative: {w}")))?;
    let height =
        usize::try_from(h).map_err(|_| malformed(format!("output height is negative: {h}")))?;
    let plane = width
        .checked_mul(height)
        .ok_or_else(|| malformed(format!("output plane dimensions overflow usize: {w}x{h}")))?;
    let (shape, channels) = match dims {
        1 => (vec![w], 1usize),
        2 => (vec![h, w], 1usize),
        3 => (
            vec![c, h, w],
            usize::try_from(c)
                .map_err(|_| malformed(format!("output channel count is negative: {c}")))?,
        ),
        other => return Err(malformed(format!("unexpected output dims {other}"))),
    };
    let total = plane
        .checked_mul(channels)
        .ok_or_else(|| malformed("output element count overflows usize".to_string()))?;
    let mut values = Vec::with_capacity(total);
    for channel in 0..channels {
        // SAFETY: elemsize == 4 and elempack == 1 give an aligned f32 array;
        // channel < c with ncnn's own cstep keeps each slice inside it.
        unsafe {
            let start = data.cast::<f32>().add(channel * cstep);
            values.extend_from_slice(std::slice::from_raw_parts(start, plane));
        }
    }
    Ok(Output {
        shape,
        data: values,
    })
}

fn path_to_cstring(path: &Path) -> Result<CString, Error> {
    let text = path.to_str().ok_or(Error::InvalidPath)?;
    CString::new(text).map_err(|_| Error::InvalidPath)
}
