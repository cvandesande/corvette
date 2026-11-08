//! A generated net: out0 is in0 plus a constant, with the detection model's recorded blob names
//! and its 8,400 x 50 element count. The [1, 50, 8400] shape is this net's own layout, not a
//! fact about the deployed model.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use corvette_ncnn::{Device, Error, InputShape, LoadStage, Net, Output, live_handles};

const WIDTH: i32 = 8400;
const HEIGHT: i32 = 50;
const CHANNELS: i32 = 1;
const ELEMENTS: usize = 420_000;

const SHAPE: InputShape = InputShape {
    width: WIDTH,
    height: HEIGHT,
    channels: CHANNELS,
};

const PARAM: &str = concat!(
    "7767517\n",
    "3 3\n",
    "Input in0 0 1 in0 0=8400 1=50 2=1\n",
    "MemoryData bias 0 1 bias 0=8400 1=50 2=1\n",
    "BinaryOp add 2 1 in0 bias out0 0=0\n",
);

const TRUNCATED_BIN_BYTES: usize = 100;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

// Every value cycles through small integers or exact quarters, so comparisons need no tolerance.
fn cycled<const N: u8>(f: impl Fn(u8) -> f32) -> Vec<f32> {
    (0u8..N).cycle().take(ELEMENTS).map(f).collect()
}

fn constant_values() -> Vec<f32> {
    cycled::<97>(|v| f32::from(v) * 0.25)
}

fn input_values() -> Vec<f32> {
    cycled::<13>(|v| f32::from(v) - 6.0)
}

fn second_input_values() -> Vec<f32> {
    cycled::<7>(f32::from)
}

fn expected(input: &[f32], constant: &[f32]) -> Vec<f32> {
    input.iter().zip(constant).map(|(a, b)| a + b).collect()
}

fn le_f32(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn write_net(tag: &str, param: &str, bin: &[u8]) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    fs::create_dir_all(&dir).expect("the net directory is creatable");
    let param_path = dir.join("net.param");
    let bin_path = dir.join("net.bin");
    fs::write(&param_path, param).expect("the param file is writable");
    fs::write(&bin_path, bin).expect("the weights file is writable");
    (param_path, bin_path)
}

fn assert_tensor(output: &Output, want: &[f32]) {
    assert_eq!(output.data.len(), want.len(), "output length differs");
    for (i, (got, want)) in output.data.iter().zip(want).enumerate() {
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "out0 element {i} differs: {got} vs {want}"
        );
    }
}

#[test]
fn synthetic_net_output_is_input_plus_constant() {
    let _serial = serial();
    let bias = constant_values();
    let mut input = input_values();
    let want = expected(&input, &bias);
    let (param, bin) = write_net("input-plus-constant", PARAM, &le_f32(&bias));
    let net = Net::load(&param, &bin, Device::Cpu, SHAPE).expect("the synthetic net loads");
    assert_eq!(net.input_name(), "in0");
    assert_eq!(net.output_name(), "out0");
    let output = net.infer(&mut input).expect("infer succeeds");
    assert_eq!(output.shape, vec![CHANNELS, HEIGHT, WIDTH]);
    assert_eq!(output.data.len(), ELEMENTS);
    assert_tensor(&output, &want);
}

#[test]
fn synthetic_net_is_stable_across_repeated_inference() {
    let _serial = serial();
    let before = live_handles();
    let bias = constant_values();
    let mut first_input = input_values();
    let want_first = expected(&first_input, &bias);
    let mut second_input = second_input_values();
    let want_second = expected(&second_input, &bias);
    let (param, bin) = write_net("repeated-inference", PARAM, &le_f32(&bias));
    let net = Net::load(&param, &bin, Device::Cpu, SHAPE).expect("the synthetic net loads");
    let first_output = net
        .infer(&mut first_input)
        .expect("the first infer succeeds");
    assert_tensor(&first_output, &want_first);
    let second_output = net
        .infer(&mut second_input)
        .expect("the second infer succeeds");
    assert_tensor(&second_output, &want_second);
    let mid = live_handles();
    assert_eq!(mid.extractors, 0, "infer left an extractor behind");
    assert_eq!(mid.mats, 0, "infer left a Mat behind");
    drop(net);
    assert_eq!(live_handles(), before, "the net did not release its handle");
}

#[test]
fn a_corrupt_param_is_a_param_load_error() {
    let _serial = serial();
    let before = live_handles();
    let (param, bin) = write_net("corrupt-param", "not a param file\n", &[]);
    match Net::load(&param, &bin, Device::Cpu, SHAPE) {
        Err(Error::Load { stage, path, .. }) => {
            assert_eq!(stage, LoadStage::Param);
            assert_eq!(path, param);
        }
        other => panic!("expected Error::Load with stage Param, got {other:?}"),
    }
    assert_eq!(live_handles(), before, "a failed load leaked a handle");
}

#[test]
fn a_truncated_weights_file_is_a_model_load_error() {
    let _serial = serial();
    let before = live_handles();
    let bias = le_f32(&constant_values());
    let (param, bin) = write_net("truncated-weights", PARAM, &bias[..TRUNCATED_BIN_BYTES]);
    match Net::load(&param, &bin, Device::Cpu, SHAPE) {
        Err(Error::Load { stage, path, .. }) => {
            assert_eq!(stage, LoadStage::Model);
            assert_eq!(path, bin);
        }
        other => panic!("expected Error::Load with stage Model, got {other:?}"),
    }
    assert_eq!(live_handles(), before, "a failed load leaked a handle");
}

#[test]
fn the_wrong_input_length_is_refused_for_the_detection_shape() {
    let _serial = serial();
    let bias = constant_values();
    let (param, bin) = write_net("input-length", PARAM, &le_f32(&bias));
    let net = Net::load(&param, &bin, Device::Cpu, SHAPE).expect("the synthetic net loads");
    let mut short = vec![0.0f32; ELEMENTS - 1];
    match net.infer(&mut short) {
        Err(Error::InputLength { expected, actual }) => {
            assert_eq!(expected, ELEMENTS);
            assert_eq!(actual, ELEMENTS - 1);
        }
        other => panic!("expected Error::InputLength, got {other:?}"),
    }
}
