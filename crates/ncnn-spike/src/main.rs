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
//!   `MODEL_PARAM`   .param file; the .bin is derived from it (required)
//!   `MODEL_SIZE`    square input resolution (default 320)
//!   `BENCH_ITERS`   timed iterations (default 2000)
//!   `NCNN_DEVICE`   Vulkan device index (default: ncnn's own choice)
//!   `INPUT_F32`     raw f32 input, 3*size*size little-endian (default: zeros)
//!   `OUTPUT_F32`    where to write the raw f32 output tensor
//!   `ALLOW_CPU`     accept a Vulkan device that reports type=cpu

use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

use ncnn_sys::{
    DeviceType, ncnn_ext_destroy_gpu_instance, ncnn_ext_get_default_gpu_index,
    ncnn_ext_get_gpu_count, ncnn_ext_get_gpu_device_name, ncnn_ext_get_gpu_driver_name,
    ncnn_ext_get_gpu_rough_score, ncnn_ext_get_gpu_type, ncnn_extractor_create,
    ncnn_extractor_destroy, ncnn_extractor_extract, ncnn_extractor_input,
    ncnn_mat_create_external_3d, ncnn_mat_destroy, ncnn_mat_get_c, ncnn_mat_get_cstep,
    ncnn_mat_get_data, ncnn_mat_get_dims, ncnn_mat_get_elempack, ncnn_mat_get_elemsize,
    ncnn_mat_get_h, ncnn_mat_get_w, ncnn_mat_t, ncnn_net_create, ncnn_net_destroy,
    ncnn_net_get_input_count, ncnn_net_get_input_name, ncnn_net_get_option,
    ncnn_net_get_output_count, ncnn_net_get_output_name, ncnn_net_load_model, ncnn_net_load_param,
    ncnn_net_set_vulkan_device, ncnn_net_t, ncnn_option_get_use_vulkan_compute,
    ncnn_option_set_use_fp16_arithmetic, ncnn_option_set_use_fp16_packed,
    ncnn_option_set_use_fp16_storage, ncnn_option_set_use_vulkan_compute, ncnn_version,
};

type Failure = String;

/// Owns the net so an early return cannot leak it.
struct Net(ncnn_net_t);

impl Drop for Net {
    fn drop(&mut self) {
        // SAFETY: self.0 came from ncnn_net_create and is dropped once, and no
        // extractor outlives the net -- infer() destroys each one it makes.
        unsafe { ncnn_net_destroy(self.0) }
    }
}

/// Owns an extractor for the duration of one inference.
struct Extractor(ncnn_sys::ncnn_extractor_t);

impl Drop for Extractor {
    fn drop(&mut self) {
        // SAFETY: self.0 came from ncnn_extractor_create and is dropped once.
        unsafe { ncnn_extractor_destroy(self.0) }
    }
}

/// Owns a Mat while preserving any separately owned backing buffer.
struct Mat(ncnn_mat_t);

impl Drop for Mat {
    fn drop(&mut self) {
        // SAFETY: self.0 came from an ncnn Mat constructor or extractor and is
        // dropped once. ncnn does not free external backing storage here.
        unsafe { ncnn_mat_destroy(self.0) }
    }
}

struct Config {
    param_path: String,
    bin_path: String,
    size: c_int,
    elements: usize,
    iters: usize,
}

struct BlobNames {
    input: CString,
    output: CString,
}

struct Stats {
    head_mean: f64,
    steady_mean: f64,
    steady_median: f64,
    steady_p95: f64,
    steady_min: f64,
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> Result<T, Failure> {
    env::var(key).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("{key}: cannot parse {value:?}"))
    })
}

/// Copies a C string ncnn owns into a `String`.
///
/// # Safety
/// `ptr` must be null, or point to a NUL-terminated string that stays valid
/// for the duration of the call.
unsafe fn cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return "<null>".into();
    }
    // SAFETY: non-null by the check above, and the caller guarantees the
    // pointee is a valid NUL-terminated string.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

// The four helpers below are the whole of csrc/c_api_ext.cpp. Each is sound
// for any argument: the shim bounds-checks the index itself and reports a miss
// as -1/0/null rather than reading past the device array.
fn gpu_count() -> c_int {
    // SAFETY: no arguments, and the shim creates the Vulkan instance on first
    // call rather than assuming one exists.
    unsafe { ncnn_ext_get_gpu_count() }
}

fn device_name(index: c_int) -> String {
    // SAFETY: the shim returns null for an out-of-range index, which cstr
    // handles, and any string it does return is owned by ncnn for the life of
    // the Vulkan instance -- which outlives this call.
    unsafe { cstr(ncnn_ext_get_gpu_device_name(index)) }
}

fn driver_name(index: c_int) -> String {
    // SAFETY: as device_name above.
    unsafe { cstr(ncnn_ext_get_gpu_driver_name(index)) }
}

fn default_device() -> c_int {
    // SAFETY: no arguments; returns -1 when nothing enumerated, which the
    // caller's range check rejects.
    unsafe { ncnn_ext_get_default_gpu_index() }
}

fn device_type(index: c_int) -> DeviceType {
    // SAFETY: the shim bounds-checks and returns -1 out of range, which maps
    // to DeviceType::Unknown.
    DeviceType::from_raw(unsafe { ncnn_ext_get_gpu_type(index) })
}

/// Enumerate, then select. Only selection is in upstream's C API; everything
/// this function prints comes from `csrc/c_api_ext.cpp`.
fn select_device() -> Result<c_int, Failure> {
    let count = gpu_count();
    if count <= 0 {
        return Err("no Vulkan device enumerated".into());
    }
    for index in 0..count {
        // SAFETY: bounds-checked by the shim, as above.
        let score = unsafe { ncnn_ext_get_gpu_rough_score(index) };
        eprintln!(
            "ncnn-spike: vulkan device {index}: {} [driver={} type={} score={score}]",
            device_name(index),
            driver_name(index),
            device_type(index).as_str(),
        );
    }

    let device = env::var("NCNN_DEVICE").map_or_else(
        |_| Ok(default_device()),
        |value| {
            value
                .parse::<c_int>()
                .map_err(|_| format!("NCNN_DEVICE: cannot parse {value:?}"))
        },
    )?;
    if !(0..count).contains(&device) {
        return Err(format!(
            "device {device} out of range, {count} Vulkan device(s) present"
        ));
    }

    // The reason the enumeration gap mattered. A CPU-type device is lavapipe:
    // it is a working Vulkan path at a fraction of the speed, and without
    // GpuInfo::type() the only way to notice is to match on the device name.
    if device_type(device) == DeviceType::Cpu && env::var("ALLOW_CPU").is_err() {
        return Err(format!(
            "device {device} ({}) is a software rasterizer; set ALLOW_CPU=1 to use it deliberately",
            device_name(device)
        ));
    }
    eprintln!(
        "ncnn-spike: using vulkan device {device} ({})",
        device_name(device)
    );
    Ok(device)
}

/// Creates the net, pins the device and the fp32 options, and loads the model.
/// Returns the load time in milliseconds alongside it.
fn load_net(config: &Config, device: c_int) -> Result<(Net, f64), Failure> {
    // SAFETY: ncnn_net_create returns an owned handle or null; the option
    // pointer borrows the net and is only read/written while the net is alive.
    // Every setter must precede load_param, which is why they are in one block
    // with it.
    let net = unsafe {
        let handle = ncnn_net_create();
        if handle.is_null() {
            return Err("ncnn_net_create returned null".into());
        }
        let net = Net(handle);
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
        net
    };

    let param = CString::new(config.param_path.clone()).map_err(|e| e.to_string())?;
    let bin = CString::new(config.bin_path.clone()).map_err(|e| e.to_string())?;
    let start = Instant::now();
    // SAFETY: both pointers are valid NUL-terminated paths owned by the
    // CStrings above, which outlive the calls.
    unsafe {
        // Both return -1 rather than raising, so an unchecked load defers the
        // failure to the first inference, where it looks like a lost device.
        let rc = ncnn_net_load_param(net.0, param.as_ptr());
        if rc != 0 {
            return Err(format!(
                "failed to parse parameter file {} ({rc})",
                config.param_path
            ));
        }
        let rc = ncnn_net_load_model(net.0, bin.as_ptr());
        if rc != 0 {
            return Err(format!("failed to load weights {} ({rc})", config.bin_path));
        }
    }
    Ok((net, start.elapsed().as_secs_f64() * 1000.0))
}

/// The C API names blobs directly, so the `.param` parsing `ncnn.py` does by
/// hand is not needed on this path -- but honour an override in case a model's
/// declared outputs ever disagree with its last layer.
fn blob_names(net: &Net) -> Result<BlobNames, Failure> {
    // SAFETY: the net is loaded, so its blob tables are populated; the
    // returned strings are owned by the net, which outlives this call.
    let (input, output, inputs, outputs) = unsafe {
        (
            cstr(ncnn_net_get_input_name(net.0, 0)),
            cstr(ncnn_net_get_output_name(net.0, 0)),
            ncnn_net_get_input_count(net.0),
            ncnn_net_get_output_count(net.0),
        )
    };
    let input = env::var("INPUT_BLOB").unwrap_or(input);
    let output = env::var("OUTPUT_BLOB").unwrap_or(output);
    eprintln!("ncnn-spike: blobs in={input} ({inputs} declared) out={output} ({outputs} declared)");
    Ok(BlobNames {
        input: CString::new(input)
            .map_err(|_| "INPUT_BLOB contains an interior NUL byte".to_string())?,
        output: CString::new(output)
            .map_err(|_| "OUTPUT_BLOB contains an interior NUL byte".to_string())?,
    })
}

/// One inference, structured like `NcnnDetector._extract`: a fresh extractor
/// and a Mat borrowing the caller's buffer, which therefore has to outlive it.
fn infer(
    net: &Net,
    blob_names: &BlobNames,
    input: &mut [f32],
    size: c_int,
) -> Result<(Vec<f32>, Vec<c_int>), Failure> {
    // SAFETY: the Mat borrows `input`, which is borrowed mutably for the whole
    // block and so cannot move or be freed; every handle created here is
    // destroyed on every path out. The extract() output Mat is ours to destroy
    // per the C API.
    unsafe {
        let mat_handle =
            ncnn_mat_create_external_3d(size, size, 3, input.as_mut_ptr().cast(), ptr::null_mut());
        if mat_handle.is_null() {
            return Err("ncnn_mat_create_external_3d returned null".into());
        }
        let mat = Mat(mat_handle);
        let extractor_handle = ncnn_extractor_create(net.0);
        if extractor_handle.is_null() {
            return Err("ncnn_extractor_create returned null".into());
        }
        let extractor = Extractor(extractor_handle);
        let rc = ncnn_extractor_input(extractor.0, blob_names.input.as_ptr(), mat.0);
        if rc != 0 {
            return Err(format!(
                "set extractor input {:?}: ncnn returned {rc}",
                blob_names.input
            ));
        }
        let mut output_handle: ncnn_mat_t = ptr::null_mut();
        let rc = ncnn_extractor_extract(
            extractor.0,
            blob_names.output.as_ptr(),
            &raw mut output_handle,
        );
        let output = (!output_handle.is_null()).then(|| Mat(output_handle));
        if rc != 0 {
            // Frigate treats a nonzero extract as a lost device, which only a
            // new process recovers from; the spike just reports it.
            return Err(format!(
                "extract output {:?}: ncnn returned {rc}",
                blob_names.output
            ));
        }
        let Some(output) = output else {
            return Err(format!(
                "extract output {:?}: ncnn returned a null Mat",
                blob_names.output
            ));
        };
        copy_out(output.0)
    }
}

/// Flattens an output Mat the way `np.array(mat)` does: `[w]`, `[h, w]` or
/// `[c, h, w]`, walking `cstep` rather than assuming channels are contiguous.
///
/// # Safety
/// `mat` must be a live Mat returned by `ncnn_extractor_extract`.
unsafe fn copy_out(mat: ncnn_mat_t) -> Result<(Vec<f32>, Vec<c_int>), Failure> {
    // SAFETY: the caller guarantees `mat` is live; all of these are pure
    // accessors on it.
    let (elemsize, elempack, dims, w, h, c, cstep, output_data) = unsafe {
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
    // Guards the cast below: 4-byte elements, one per slot, is plain fp32.
    if elemsize != 4 || elempack != 1 {
        return Err(format!(
            "expected unpacked fp32 output, got elemsize={elemsize} elempack={elempack}"
        ));
    }
    if output_data.is_null() {
        return Err("output Mat has no data".into());
    }

    let width = usize::try_from(w).map_err(|_| format!("output width is negative: {w}"))?;
    let height = usize::try_from(h).map_err(|_| format!("output height is negative: {h}"))?;
    let plane_elements = width
        .checked_mul(height)
        .ok_or_else(|| format!("output plane dimensions overflow usize: {w}x{h}"))?;
    let (shape, plane, channels) = match dims {
        1 => (vec![w], width, 1usize),
        2 => (vec![h, w], plane_elements, 1usize),
        3 => (
            vec![c, h, w],
            plane_elements,
            usize::try_from(c).map_err(|_| format!("output channel count is negative: {c}"))?,
        ),
        other => return Err(format!("unexpected output dims {other}")),
    };

    let output_elements = plane
        .checked_mul(channels)
        .ok_or_else(|| "output element count overflows usize".to_string())?;
    let mut output_values = Vec::with_capacity(output_elements);
    for channel in 0..channels {
        // SAFETY: elemsize == 4 and elempack == 1 make this an array of f32,
        // which ncnn allocates with at least that alignment; channel < c and
        // cstep is ncnn's own channel stride, so each slice stays inside the
        // allocation.
        unsafe {
            let start = output_data.cast::<f32>().add(channel * cstep);
            output_values.extend_from_slice(std::slice::from_raw_parts(start, plane));
        }
    }
    Ok((output_values, shape))
}

fn mean(values: &[f64]) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "iteration counts are nowhere near 2^53"
    )]
    let count = values.len() as f64;
    values.iter().sum::<f64>() / count
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        f64::midpoint(sorted[n / 2 - 1], sorted[n / 2])
    }
}

/// Same quartile split as `bench_steady.py`: a head that runs at the boosted
/// rate, and the sustained tail Frigate actually lives with.
fn summarise(samples: &[f64], iters: usize) -> Stats {
    let quarter = std::cmp::max(1, iters / 4);
    let head = &samples[..quarter];
    let tail_count = std::cmp::max(1, iters / 2);
    let tail = &samples[samples.len() - tail_count..];
    let mut sorted = tail.to_vec();
    sorted.sort_by(f64::total_cmp);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "matches bench_steady.py's int(len * 0.95) exactly, and the index is in range"
    )]
    let p95 = sorted[(sorted.len() as f64 * 0.95) as usize];
    Stats {
        head_mean: mean(head),
        steady_mean: mean(tail),
        steady_median: median(&sorted),
        steady_p95: p95,
        steady_min: sorted[0],
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

fn config() -> Result<Config, Failure> {
    let param_path = env::var("MODEL_PARAM").map_err(|_| "MODEL_PARAM is not set".to_string())?;
    let bin_path = param_path
        .strip_suffix(".param")
        .map(|stem| format!("{stem}.bin"))
        .ok_or_else(|| format!("MODEL_PARAM must name a .param file, got {param_path}"))?;
    let size: c_int = env_or("MODEL_SIZE", 320)?;
    let extent = usize::try_from(size)
        .ok()
        .filter(|extent| *extent > 0)
        .ok_or_else(|| format!("MODEL_SIZE must be positive, got {size}"))?;
    let elements = extent
        .checked_mul(extent)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| format!("MODEL_SIZE is too large: {size}"))?;
    let iters: usize = env_or("BENCH_ITERS", 2000)?;
    if iters == 0 {
        return Err("BENCH_ITERS must be positive, got 0".to_string());
    }
    Ok(Config {
        param_path,
        bin_path,
        size,
        elements,
        iters,
    })
}

fn read_input(config: &Config) -> Result<Vec<f32>, Failure> {
    let Ok(path) = env::var("INPUT_F32") else {
        return Ok(vec![0.0f32; config.elements]);
    };
    let bytes = fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.len() != config.elements * 4 {
        return Err(format!(
            "{path}: expected {} bytes for 3x{size}x{size} f32, got {}",
            config.elements * 4,
            bytes.len(),
            size = config.size,
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn run() -> Result<(), Failure> {
    let config = config()?;
    // SAFETY: no arguments, and the returned string is static in ncnn.
    eprintln!("ncnn-spike: ncnn {}", unsafe { cstr(ncnn_version()) });
    let device = select_device()?;
    let (net, load_ms) = load_net(&config, device)?;
    let blob_names = blob_names(&net)?;
    let mut input = read_input(&config)?;

    let (first, shape) = infer(&net, &blob_names, &mut input, config.size)?;
    if let Ok(path) = env::var("OUTPUT_F32") {
        let mut bytes = Vec::with_capacity(first.len() * 4);
        for value in &first {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))?;
    }

    let mut samples = Vec::with_capacity(config.iters);
    for _ in 0..config.iters {
        let start = Instant::now();
        infer(&net, &blob_names, &mut input, config.size)?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let stats = summarise(&samples, config.iters);

    println!(
        concat!(
            r#"RESULT {{"model": "{}", "size": {}, "device": "{}", "device_type": "{}", "#,
            r#""input_blob": "{}", "output_blob": "{}", "load_ms": {}, "output_shape": {:?}, "#,
            r#""output_len": {}, "iters": {}, "head_mean_ms": {}, "steady_mean_ms": {}, "#,
            r#""steady_median_ms": {}, "steady_p95_ms": {}, "steady_min_ms": {}, "steady_fps": {}}}"#
        ),
        Path::new(&config.param_path).file_name().map_or_else(
            || config.param_path.clone(),
            |n| n.to_string_lossy().into_owned()
        ),
        config.size,
        device_name(device),
        device_type(device).as_str(),
        blob_names.input.to_string_lossy(),
        blob_names.output.to_string_lossy(),
        round(load_ms, 1),
        shape,
        first.len(),
        config.iters,
        round(stats.head_mean, 3),
        round(stats.steady_mean, 3),
        round(stats.steady_median, 3),
        round(stats.steady_p95, 3),
        round(stats.steady_min, 3),
        round(1000.0 / stats.steady_mean, 1),
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            // ncnn's Vulkan instance is process-wide; tearing it down here
            // keeps validation-layer runs quiet about leaked devices.
            // SAFETY: no arguments, and nothing holds a device afterwards --
            // the net was dropped when run() returned.
            unsafe { ncnn_ext_destroy_gpu_instance() };
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("ncnn-spike: {message}");
            ExitCode::from(2)
        }
    }
}
