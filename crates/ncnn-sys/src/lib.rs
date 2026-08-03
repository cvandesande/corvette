//! Raw FFI over ncnn's C API (`src/c_api.h`), plus `c_api_ext` for the GPU
//! enumeration the C API does not expose.
//!
//! Only the surface the detector actually uses is declared, so this file
//! doubles as the answer to "is the C API wide enough?" -- everything the
//! Python plugin calls appears below, and the four entry points prefixed
//! `ncnn_ext_` are the ones we had to add.
//!
//! Every handle is an opaque pointer in ncnn's ABI, so no struct layouts are
//! reproduced here and nothing can drift out of sync with a version bump
//! except the function signatures themselves.
#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

pub enum __ncnn_net_t {}
pub enum __ncnn_option_t {}
pub enum __ncnn_mat_t {}
pub enum __ncnn_extractor_t {}
pub enum __ncnn_allocator_t {}

pub type ncnn_net_t = *mut __ncnn_net_t;
pub type ncnn_option_t = *mut __ncnn_option_t;
pub type ncnn_mat_t = *mut __ncnn_mat_t;
pub type ncnn_extractor_t = *mut __ncnn_extractor_t;
pub type ncnn_allocator_t = *mut __ncnn_allocator_t;

extern "C" {
    pub fn ncnn_version() -> *const c_char;

    pub fn ncnn_net_create() -> ncnn_net_t;
    pub fn ncnn_net_destroy(net: ncnn_net_t);
    /// Borrows the net's own `Option`; it must not be passed to
    /// `ncnn_option_destroy`, which would free storage the net still owns.
    pub fn ncnn_net_get_option(net: ncnn_net_t) -> ncnn_option_t;
    pub fn ncnn_net_set_vulkan_device(net: ncnn_net_t, device_index: c_int);
    pub fn ncnn_net_load_param(net: ncnn_net_t, path: *const c_char) -> c_int;
    pub fn ncnn_net_load_model(net: ncnn_net_t, path: *const c_char) -> c_int;
    pub fn ncnn_net_get_input_count(net: ncnn_net_t) -> c_int;
    pub fn ncnn_net_get_output_count(net: ncnn_net_t) -> c_int;
    pub fn ncnn_net_get_input_name(net: ncnn_net_t, i: c_int) -> *const c_char;
    pub fn ncnn_net_get_output_name(net: ncnn_net_t, i: c_int) -> *const c_char;

    pub fn ncnn_option_set_use_vulkan_compute(opt: ncnn_option_t, enable: c_int);
    pub fn ncnn_option_get_use_vulkan_compute(opt: ncnn_option_t) -> c_int;
    pub fn ncnn_option_set_use_fp16_packed(opt: ncnn_option_t, enable: c_int);
    pub fn ncnn_option_set_use_fp16_storage(opt: ncnn_option_t, enable: c_int);
    pub fn ncnn_option_set_use_fp16_arithmetic(opt: ncnn_option_t, enable: c_int);

    /// Borrows `data`, exactly as `ncnn.Mat(numpy_array)` borrows NumPy
    /// storage: the buffer must outlive the Mat.
    pub fn ncnn_mat_create_external_3d(
        w: c_int,
        h: c_int,
        c: c_int,
        data: *mut c_void,
        allocator: ncnn_allocator_t,
    ) -> ncnn_mat_t;
    pub fn ncnn_mat_destroy(mat: ncnn_mat_t);
    pub fn ncnn_mat_get_dims(mat: ncnn_mat_t) -> c_int;
    pub fn ncnn_mat_get_w(mat: ncnn_mat_t) -> c_int;
    pub fn ncnn_mat_get_h(mat: ncnn_mat_t) -> c_int;
    pub fn ncnn_mat_get_c(mat: ncnn_mat_t) -> c_int;
    pub fn ncnn_mat_get_elemsize(mat: ncnn_mat_t) -> usize;
    pub fn ncnn_mat_get_elempack(mat: ncnn_mat_t) -> c_int;
    pub fn ncnn_mat_get_cstep(mat: ncnn_mat_t) -> usize;
    pub fn ncnn_mat_get_data(mat: ncnn_mat_t) -> *mut c_void;

    pub fn ncnn_extractor_create(net: ncnn_net_t) -> ncnn_extractor_t;
    pub fn ncnn_extractor_destroy(ex: ncnn_extractor_t);
    pub fn ncnn_extractor_input(
        ex: ncnn_extractor_t,
        name: *const c_char,
        mat: ncnn_mat_t,
    ) -> c_int;
    pub fn ncnn_extractor_extract(
        ex: ncnn_extractor_t,
        name: *const c_char,
        mat: *mut ncnn_mat_t,
    ) -> c_int;

    // csrc/c_api_ext.cpp -- not part of upstream's C API.
    pub fn ncnn_ext_get_gpu_count() -> c_int;
    pub fn ncnn_ext_get_default_gpu_index() -> c_int;
    pub fn ncnn_ext_get_gpu_type(device_index: c_int) -> c_int;
    pub fn ncnn_ext_get_gpu_device_name(device_index: c_int) -> *const c_char;
    pub fn ncnn_ext_get_gpu_driver_name(device_index: c_int) -> *const c_char;
    pub fn ncnn_ext_get_gpu_rough_score(device_index: c_int) -> c_uint;
    pub fn ncnn_ext_destroy_gpu_instance();
}

/// `GpuInfo::type()`. The software rasterizer is the reason this is exposed:
/// lavapipe reports `Cpu` while every real target reports `Discrete` or
/// `Integrated`, which is a structural test rather than a name match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Unknown(i32),
}

impl DeviceType {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => DeviceType::Discrete,
            1 => DeviceType::Integrated,
            2 => DeviceType::Virtual,
            3 => DeviceType::Cpu,
            other => DeviceType::Unknown(other),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeviceType::Discrete => "discrete",
            DeviceType::Integrated => "integrated",
            DeviceType::Virtual => "virtual",
            DeviceType::Cpu => "cpu",
            DeviceType::Unknown(_) => "unknown",
        }
    }
}
