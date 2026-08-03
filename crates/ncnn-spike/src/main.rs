//! The ncnn-from-Rust spike.
//!
//! Runs exactly what `docker/frigate/ncnn.py` runs -- same model, same
//! options, same per-iteration extractor/Mat churn -- over ncnn's C API, and
//! reports the same JSON `scripts/bench_steady.py` reports so the two can be
//! compared directly. With `INPUT_F32`/`OUTPUT_F32` set it also writes the raw
//! output tensor, which is what turns "it runs" into "it computes the same
//! thing".
//!
//! Environment:
//!   MODEL_PARAM   .param file; the .bin is derived from it (required)
//!   MODEL_SIZE    square input resolution (default 320)
//!   BENCH_ITERS   timed iterations (default 2000)
//!   NCNN_DEVICE   Vulkan device index (default: ncnn's own choice)
//!   INPUT_F32     raw f32 input, 3*size*size little-endian (default: zeros)
//!   OUTPUT_F32    where to write the raw f32 output tensor
//!   ALLOW_CPU     accept a Vulkan device that reports type=cpu

use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::c_void;
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

use ncnn_sys::*;

struct Net(ncnn_net_t);

impl Drop for Net {
    fn drop(&mut self) {
        unsafe { ncnn_net_destroy(self.0) }
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("{key}: cannot parse {value:?}")),
        Err(_) => default,
    }
}

fn cstr(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return "<null>".into();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

/// Enumerate, then select. Selection alone is in upstream's C API; everything
/// this function prints comes from csrc/c_api_ext.cpp.
fn select_device() -> Result<i32, String> {
    let count = unsafe { ncnn_ext_get_gpu_count() };
    if count <= 0 {
        return Err("no Vulkan device enumerated".into());
    }
    for index in 0..count {
        let kind = DeviceType::from_raw(unsafe { ncnn_ext_get_gpu_type(index) });
        eprintln!(
            "ncnn-spike: vulkan device {index}: {} [driver={} type={} score={}]",
            cstr(unsafe { ncnn_ext_get_gpu_device_name(index) }),
            cstr(unsafe { ncnn_ext_get_gpu_driver_name(index) }),
            kind.as_str(),
            unsafe { ncnn_ext_get_gpu_rough_score(index) },
        );
    }

    let device = match env::var("NCNN_DEVICE") {
        Ok(value) => value
            .parse::<i32>()
            .map_err(|_| format!("NCNN_DEVICE: cannot parse {value:?}"))?,
        Err(_) => unsafe { ncnn_ext_get_default_gpu_index() },
    };
    if !(0..count).contains(&device) {
        return Err(format!(
            "device {device} out of range, {count} Vulkan device(s) present"
        ));
    }

    // The reason the enumeration gap mattered. A CPU-type device is lavapipe:
    // it is a working Vulkan path at a fraction of the speed, and without
    // GpuInfo::type() the only way to notice is to match on the device name.
    let kind = DeviceType::from_raw(unsafe { ncnn_ext_get_gpu_type(device) });
    if kind == DeviceType::Cpu && env::var("ALLOW_CPU").is_err() {
        return Err(format!(
            "device {device} ({}) is a software rasterizer; set ALLOW_CPU=1 to use it deliberately",
            cstr(unsafe { ncnn_ext_get_gpu_device_name(device) })
        ));
    }
    eprintln!(
        "ncnn-spike: using vulkan device {device} ({})",
        cstr(unsafe { ncnn_ext_get_gpu_device_name(device) })
    );
    Ok(device)
}

/// One inference, structured like `NcnnDetector._extract`: a fresh extractor
/// and a Mat borrowing the caller's buffer, which therefore has to outlive it.
fn infer(net: &Net, input_name: &CStr, output_name: &CStr, input: &mut [f32], size: i32) -> Result<(Vec<f32>, Vec<i32>), String> {
    unsafe {
        let mat = ncnn_mat_create_external_3d(
            size,
            size,
            3,
            input.as_mut_ptr() as *mut c_void,
            ptr::null_mut(),
        );
        if mat.is_null() {
            return Err("ncnn_mat_create_external_3d returned null".into());
        }
        let ex = ncnn_extractor_create(net.0);
        let rc = ncnn_extractor_input(ex, input_name.as_ptr(), mat);
        if rc != 0 {
            ncnn_extractor_destroy(ex);
            ncnn_mat_destroy(mat);
            return Err(format!("extractor input returned {rc}"));
        }
        let mut out: ncnn_mat_t = ptr::null_mut();
        let rc = ncnn_extractor_extract(ex, output_name.as_ptr(), &mut out);
        if rc != 0 || out.is_null() {
            ncnn_extractor_destroy(ex);
            ncnn_mat_destroy(mat);
            // Frigate treats a nonzero extract as a lost device, which only a
            // new process recovers from; the spike just reports it.
            return Err(format!("extract returned {rc}"));
        }

        let result = copy_out(out);
        ncnn_mat_destroy(out);
        ncnn_extractor_destroy(ex);
        ncnn_mat_destroy(mat);
        result
    }
}

/// Flatten an output Mat the way `np.array(mat)` does: [w], [h,w] or [c,h,w],
/// walking `cstep` rather than assuming channels are contiguous.
unsafe fn copy_out(mat: ncnn_mat_t) -> Result<(Vec<f32>, Vec<i32>), String> {
    let elemsize = ncnn_mat_get_elemsize(mat);
    let elempack = ncnn_mat_get_elempack(mat);
    if elemsize != 4 || elempack != 1 {
        return Err(format!(
            "expected unpacked fp32 output, got elemsize={elemsize} elempack={elempack}"
        ));
    }
    let (dims, w, h, c) = (
        ncnn_mat_get_dims(mat),
        ncnn_mat_get_w(mat),
        ncnn_mat_get_h(mat),
        ncnn_mat_get_c(mat),
    );
    let cstep = ncnn_mat_get_cstep(mat);
    let data = ncnn_mat_get_data(mat) as *const f32;
    if data.is_null() {
        return Err("output Mat has no data".into());
    }

    let (shape, plane) = match dims {
        1 => (vec![w], w as usize),
        2 => (vec![h, w], (w * h) as usize),
        3 => (vec![c, h, w], (w * h) as usize),
        other => return Err(format!("unexpected output dims {other}")),
    };
    let channels = if dims == 3 { c as usize } else { 1 };
    let mut out = Vec::with_capacity(plane * channels);
    for channel in 0..channels {
        let start = data.add(channel * cstep);
        out.extend_from_slice(std::slice::from_raw_parts(start, plane));
    }
    Ok((out, shape))
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    sorted[(sorted.len() as f64 * q) as usize]
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn run() -> Result<(), String> {
    let param_path = env::var("MODEL_PARAM").map_err(|_| "MODEL_PARAM is not set".to_string())?;
    let bin_path = param_path
        .strip_suffix(".param")
        .map(|stem| format!("{stem}.bin"))
        .ok_or_else(|| format!("MODEL_PARAM must name a .param file, got {param_path}"))?;
    let size: i32 = env_or("MODEL_SIZE", 320);
    let iters: usize = env_or("BENCH_ITERS", 2000);

    eprintln!("ncnn-spike: ncnn {}", cstr(unsafe { ncnn_version() }));
    let device = select_device()?;

    let net = Net(unsafe { ncnn_net_create() });
    unsafe {
        // The net's own Option -- mutating it in place, which is what the
        // Python plugin does through net.opt, so it must precede load_param.
        let opt = ncnn_net_get_option(net.0);
        ncnn_option_set_use_vulkan_compute(opt, 1);
        ncnn_net_set_vulkan_device(net.0, device);
        // Explicit fp32 everywhere, matching bench_steady.py.
        ncnn_option_set_use_fp16_packed(opt, 0);
        ncnn_option_set_use_fp16_storage(opt, 0);
        ncnn_option_set_use_fp16_arithmetic(opt, 0);
        if ncnn_option_get_use_vulkan_compute(opt) != 1 {
            return Err("use_vulkan_compute did not stick".into());
        }
    }

    let param_c = CString::new(param_path.clone()).unwrap();
    let bin_c = CString::new(bin_path.clone()).unwrap();
    let load_start = Instant::now();
    unsafe {
        // Both return -1 rather than raising, so an unchecked load defers the
        // failure to the first inference, where it looks like a lost device.
        let rc = ncnn_net_load_param(net.0, param_c.as_ptr());
        if rc != 0 {
            return Err(format!("failed to parse parameter file {param_path} ({rc})"));
        }
        let rc = ncnn_net_load_model(net.0, bin_c.as_ptr());
        if rc != 0 {
            return Err(format!("failed to load weights {bin_path} ({rc})"));
        }
    }
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    // The C API names blobs directly, so the .param parsing ncnn.py does by
    // hand is not needed on this path -- but honour an override in case a
    // model's declared outputs ever disagree with the last layer.
    let (input_count, output_count) =
        unsafe { (ncnn_net_get_input_count(net.0), ncnn_net_get_output_count(net.0)) };
    let input_name = match env::var("INPUT_BLOB") {
        Ok(name) => name,
        Err(_) => cstr(unsafe { ncnn_net_get_input_name(net.0, 0) }),
    };
    let output_name = match env::var("OUTPUT_BLOB") {
        Ok(name) => name,
        Err(_) => cstr(unsafe { ncnn_net_get_output_name(net.0, 0) }),
    };
    eprintln!(
        "ncnn-spike: blobs in={input_name} ({input_count} declared) out={output_name} ({output_count} declared)"
    );
    let input_c = CString::new(input_name.clone()).unwrap();
    let output_c = CString::new(output_name.clone()).unwrap();

    let expected = 3 * size as usize * size as usize;
    let mut input = match env::var("INPUT_F32") {
        Ok(path) => {
            let bytes = fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            if bytes.len() != expected * 4 {
                return Err(format!(
                    "{path}: expected {} bytes for 3x{size}x{size} f32, got {}",
                    expected * 4,
                    bytes.len()
                ));
            }
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect::<Vec<f32>>()
        }
        Err(_) => vec![0.0f32; expected],
    };

    let (first, shape) = infer(&net, &input_c, &output_c, &mut input, size)?;
    if let Ok(path) = env::var("OUTPUT_F32") {
        let mut bytes = Vec::with_capacity(first.len() * 4);
        for value in &first {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))?;
    }

    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        infer(&net, &input_c, &output_c, &mut input, size)?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    // Same quartile split as bench_steady.py: a head that runs at the boosted
    // rate and the sustained tail Frigate actually lives with.
    let quarter = std::cmp::max(1, iters / 4);
    let head = &samples[..quarter];
    let tail = &samples[samples.len() - iters / 2..];
    let mut sorted_tail = tail.to_vec();
    sorted_tail.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let steady_mean = mean(tail);

    println!(
        concat!(
            r#"RESULT {{"model": "{}", "size": {}, "device": "{}", "device_type": "{}", "#,
            r#""input_blob": "{}", "output_blob": "{}", "load_ms": {}, "output_shape": {:?}, "#,
            r#""output_len": {}, "iters": {}, "head_mean_ms": {}, "steady_mean_ms": {}, "#,
            r#""steady_median_ms": {}, "steady_p95_ms": {}, "steady_min_ms": {}, "steady_fps": {}}}"#
        ),
        std::path::Path::new(&param_path)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        size,
        cstr(unsafe { ncnn_ext_get_gpu_device_name(device) }),
        DeviceType::from_raw(unsafe { ncnn_ext_get_gpu_type(device) }).as_str(),
        input_name,
        output_name,
        (load_ms * 10.0).round() / 10.0,
        shape,
        first.len(),
        iters,
        round3(mean(head)),
        round3(steady_mean),
        round3(median(&sorted_tail)),
        round3(percentile(&sorted_tail, 0.95)),
        round3(sorted_tail[0]),
        (1000.0 / steady_mean * 10.0).round() / 10.0,
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            // ncnn's Vulkan instance is process-wide; tearing it down here
            // keeps validation-layer runs quiet about leaked devices.
            unsafe { ncnn_ext_destroy_gpu_instance() };
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("ncnn-spike: {message}");
            ExitCode::from(2)
        }
    }
}
