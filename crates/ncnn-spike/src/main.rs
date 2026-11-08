#![forbid(unsafe_code)]
//! Environment:
//!   `MODEL_PARAM`   .param file; the .bin is derived from it (required)
//!   `MODEL_SIZE`    square input resolution (default 320)
//!   `BENCH_ITERS`   timed iterations (default 2000)
//!   `NCNN_DEVICE`   Vulkan device index (default: ncnn's own choice)
//!   `INPUT_F32`     raw f32 input, 3*size*size little-endian (default: zeros)
//!   `OUTPUT_F32`    where to write the raw f32 output tensor
//!   `ALLOW_CPU`     accept a Vulkan device that reports type=cpu

use std::env;
use std::fs;
use std::os::raw::c_int;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use corvette_ncnn::{Device, Error, InputShape, Net};

type Failure = String;

struct Config {
    param_path: String,
    bin_path: String,
    size: c_int,
    elements: usize,
    iters: usize,
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

fn select_device() -> Result<(i32, String, String), Failure> {
    let devices = Device::enumerate();
    for info in &devices {
        eprintln!(
            "ncnn-spike: vulkan device {}: {} [driver={} type={} score={}]",
            info.index,
            info.name,
            info.driver,
            info.kind.as_str(),
            info.score,
        );
    }

    let requested: Option<i32> = match env::var("NCNN_DEVICE") {
        Err(_) => None,
        Ok(value) => Some(
            value
                .parse::<i32>()
                .map_err(|_| format!("NCNN_DEVICE: cannot parse {value:?}"))?,
        ),
    };
    let allow_software = env::var("ALLOW_CPU").is_ok();
    let device = Device::select(requested, allow_software).map_err(|error| match error {
        Error::SoftwareDevice { index, name } => format!(
            "device {index} ({name}) is a software rasterizer; set ALLOW_CPU=1 to use it deliberately"
        ),
        other => other.to_string(),
    })?;
    let index = match device {
        Device::Vulkan { index } => index,
        Device::Cpu => {
            return Err(
                "corvette-ncnn chose its CPU path; the spike always wants Vulkan".to_string(),
            );
        }
    };
    let info = devices
        .iter()
        .find(|info| info.index == index)
        .ok_or_else(|| format!("device {index} vanished from the enumeration"))?;
    eprintln!("ncnn-spike: using vulkan device {index} ({})", info.name);
    Ok((index, info.name.clone(), info.kind.as_str().to_string()))
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
    eprintln!("ncnn-spike: ncnn {}", corvette_ncnn::version());
    let (device, device_name, device_type) = select_device()?;
    let shape = InputShape {
        width: config.size,
        height: config.size,
        channels: 3,
    };
    let start = Instant::now();
    let mut net = Net::load(
        Path::new(&config.param_path),
        Path::new(&config.bin_path),
        Device::Vulkan { index: device },
        shape,
    )
    .map_err(|e| e.to_string())?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;

    let input_blob = env::var("INPUT_BLOB").unwrap_or_else(|_| net.input_name().to_string());
    let output_blob = env::var("OUTPUT_BLOB").unwrap_or_else(|_| net.output_name().to_string());
    if input_blob != net.input_name() || output_blob != net.output_name() {
        net = net
            .with_blobs(&input_blob, &output_blob)
            .map_err(|e| e.to_string())?;
    }
    eprintln!(
        "ncnn-spike: blobs in={} out={}",
        net.input_name(),
        net.output_name()
    );

    let mut input = read_input(&config)?;

    let first = net.infer(&mut input).map_err(|e| e.to_string())?;
    if let Ok(path) = env::var("OUTPUT_F32") {
        let mut bytes = Vec::with_capacity(first.data.len() * 4);
        for value in &first.data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))?;
    }

    let mut samples = Vec::with_capacity(config.iters);
    for _ in 0..config.iters {
        let start = Instant::now();
        net.infer(&mut input).map_err(|e| e.to_string())?;
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
        device_name,
        device_type,
        net.input_name(),
        net.output_name(),
        round(load_ms, 1),
        first.shape,
        first.data.len(),
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
            let _ = corvette_ncnn::release_gpu_instance();
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("ncnn-spike: {message}");
            ExitCode::from(2)
        }
    }
}
