//! CPU-only tests of the public API: a three-line input-only fixture net, no GPU.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use corvette_ncnn::{
    Device, Error, Fp16Snapshot, InputShape, LiveHandles, LoadStage, Net, OptionSnapshot,
    live_handles,
};

/// The fixture net is one Input blob (4x4x3 fp32); its only output is that same buffer.
const PARAM: &str = "7767517\n1 1\nInput in0 0 1 in0 0=4 1=4 2=3\n";
const SHAPE: InputShape = InputShape {
    width: 4,
    height: 4,
    channels: 3,
};

/// The live-handle counters are process-wide and tests run on several threads,
/// so every test that could create a net holds this for its whole body.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tmp(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let param = tmp(&format!("{tag}.param"));
    let bin = tmp(&format!("{tag}.bin"));
    fs::write(&param, PARAM).expect("fixture param is writable");
    fs::write(&bin, []).expect("fixture weights are writable");
    (param, bin)
}

fn load_cpu(tag: &str) -> Net {
    let (param, bin) = fixture(tag);
    Net::load(&param, &bin, Device::Cpu, SHAPE).expect("the fixture net loads")
}

fn input_values() -> Vec<f32> {
    (0u8..48).map(f32::from).collect()
}

#[test]
fn fixture_loads_with_declared_blobs_and_fp32_options() {
    let _serial = serial();
    let net = load_cpu("blobs-and-options");
    assert_eq!(net.input_name(), "in0");
    assert_eq!(net.output_name(), "in0");
    assert_eq!(net.device(), Device::Cpu);
    assert_eq!(
        net.options(),
        OptionSnapshot {
            vulkan_compute: false,
            fp16: Fp16Snapshot {
                packed: false,
                storage: false,
                arithmetic: false,
            },
        }
    );
}

#[test]
fn infer_returns_the_input_exactly() {
    let _serial = serial();
    let net = load_cpu("infer-passthrough");
    let mut input = input_values();
    let output = net.infer(&mut input).expect("infer succeeds");
    assert_eq!(output.shape, vec![3, 4, 4]);
    assert_eq!(output.data, input);
}

#[test]
fn live_handles_are_zero_after_infer_and_drop() {
    let _serial = serial();
    let before = live_handles();
    let net = load_cpu("handle-counting");
    let mut input = input_values();
    net.infer(&mut input).expect("infer succeeds");
    let mid = live_handles();
    assert_eq!(mid.extractors, 0, "infer left an extractor behind");
    assert_eq!(mid.mats, 0, "infer left a Mat behind");
    drop(net);
    let after = live_handles();
    assert_eq!(
        after,
        LiveHandles {
            nets: 0,
            extractors: 0,
            mats: 0
        }
    );
    assert_eq!(after, before);
}

#[test]
fn missing_param_file_is_a_load_param_error() {
    let _serial = serial();
    let before = live_handles();
    let (_param, bin) = fixture("missing-param");
    let absent = tmp("missing-param-absent.param");
    match Net::load(&absent, &bin, Device::Cpu, SHAPE) {
        Err(Error::Load { stage, path, code }) => {
            assert_eq!(stage, LoadStage::Param);
            assert_eq!(path, absent);
            assert_eq!(code, -1);
        }
        other => panic!("expected Error::Load with stage Param, got {other:?}"),
    }
    assert_eq!(live_handles(), before, "a failed load leaked a handle");
}

#[test]
fn missing_weights_file_is_a_load_model_error() {
    let _serial = serial();
    let before = live_handles();
    let (param, _bin) = fixture("missing-model");
    let absent = tmp("missing-model-absent.bin");
    match Net::load(&param, &absent, Device::Cpu, SHAPE) {
        Err(Error::Load { stage, path, code }) => {
            assert_eq!(stage, LoadStage::Model);
            assert_eq!(path, absent);
            assert_eq!(code, -1);
        }
        other => panic!("expected Error::Load with stage Model, got {other:?}"),
    }
    assert_eq!(live_handles(), before, "a failed load leaked a handle");
}

#[test]
fn unknown_input_blob_is_a_set_input_error() {
    let _serial = serial();
    // ncnn rejects an unknown input blob at ncnn_extractor_input, not at extract.
    let net = load_cpu("set-input-error")
        .with_blobs("nope", "in0")
        .expect("the blob names take");
    let mut input = input_values();
    match net.infer(&mut input) {
        Err(Error::SetInput { blob, code }) => {
            assert_eq!(blob, "nope");
            assert_eq!(code, -1);
        }
        other => panic!("expected Error::SetInput, got {other:?}"),
    }
}

#[test]
fn wrong_input_length_errors_without_creating_handles() {
    let _serial = serial();
    let net = load_cpu("input-length");
    let before = live_handles();
    let mut short = vec![0.0f32; 47];
    match net.infer(&mut short) {
        Err(Error::InputLength { expected, actual }) => {
            assert_eq!(expected, 48);
            assert_eq!(actual, 47);
        }
        other => panic!("expected Error::InputLength, got {other:?}"),
    }
    assert_eq!(live_handles(), before);
}

#[test]
fn unknown_output_blob_is_an_extract_error() {
    let _serial = serial();
    let net = load_cpu("extract-error")
        .with_blobs("in0", "nope")
        .expect("the blob names take");
    let mut input = input_values();
    match net.infer(&mut input) {
        Err(Error::Extract { blob, code }) => {
            assert_eq!(blob, "nope");
            assert_eq!(code, -1);
        }
        other => panic!("expected Error::Extract, got {other:?}"),
    }
}

#[test]
fn interior_nul_in_a_blob_name_is_rejected() {
    let _serial = serial();
    let net = load_cpu("nul-blob");
    assert!(matches!(
        net.with_blobs("in0", "no\0pe"),
        Err(Error::BlobName)
    ));
}

#[test]
fn release_gpu_instance_refuses_while_a_net_is_alive() {
    let _serial = serial();
    let net = load_cpu("nets-alive");
    match corvette_ncnn::release_gpu_instance() {
        Err(Error::NetsAlive { count }) => assert_eq!(count, 1),
        other => panic!("expected Error::NetsAlive with count 1, got {other:?}"),
    }
    drop(net);
}

#[test]
fn invalid_input_shapes_are_rejected() {
    let (param, bin) = fixture("bad-shape");
    for shape in [
        InputShape {
            width: 0,
            height: 4,
            channels: 3,
        },
        InputShape {
            width: 4,
            height: -1,
            channels: 3,
        },
        InputShape {
            width: 4,
            height: 4,
            channels: 0,
        },
        InputShape {
            width: i32::MAX,
            height: i32::MAX,
            channels: i32::MAX,
        },
        InputShape {
            width: 3,
            height: 3,
            channels: 3,
        },
        InputShape {
            width: 5,
            height: 2,
            channels: 2,
        },
    ] {
        assert!(
            matches!(
                Net::load(&param, &bin, Device::Cpu, shape),
                Err(Error::InvalidInputShape)
            ),
            "shape {shape:?} should be rejected"
        );
    }
}

#[test]
fn a_plane_that_is_a_multiple_of_four_loads() {
    let _serial = serial();
    // plane 4 is the smallest accepted; ncnn checks nothing about the shape at load.
    let (param, bin) = fixture("plane-of-four");
    let net = Net::load(
        &param,
        &bin,
        Device::Cpu,
        InputShape {
            width: 2,
            height: 2,
            channels: 3,
        },
    )
    .expect("2x2x3 has plane 4, so it must be accepted");
    drop(net);
}

#[cfg(unix)]
#[test]
fn a_path_ncnn_cannot_take_is_an_invalid_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let (_param, bin) = fixture("bad-path");
    let non_utf8 = Path::new(OsStr::from_bytes(b"/nonexistent/\xff.param"));
    assert!(
        matches!(
            Net::load(non_utf8, &bin, Device::Cpu, SHAPE),
            Err(Error::InvalidPath)
        ),
        "a non-UTF-8 path must be Error::InvalidPath, not rewritten"
    );
    let interior_nul = Path::new(OsStr::from_bytes(b"/nonexistent/a\0b.param"));
    assert!(
        matches!(
            Net::load(interior_nul, &bin, Device::Cpu, SHAPE),
            Err(Error::InvalidPath)
        ),
        "an interior NUL path must be Error::InvalidPath"
    );
}
