//! Engine contract, fatal exit path and socket serve loop of the Corvette detection worker.
#![forbid(unsafe_code)]

mod engine;
mod fatal;
mod ncnn;
mod serve;

pub use engine::{Engine, EngineError};
pub use fatal::{
    EXIT_DEADLINE_OVERRUN, EXIT_DEVICE_LOST, FatalReason, ProcessExit, Terminator, fatal,
};
pub use ncnn::NcnnEngine;
pub use serve::{ServeEnd, serve};

const _: () = assert!(
    corvette_detect_proto::RAW_VALUES == corvette_yolo::decode::RAW_LEN
        && corvette_detect_proto::TENSOR_BYTES == corvette_yolo::RGB_BYTES
        && corvette_detect_proto::MAX_ROWS == corvette_yolo::decode::MAX_DETECTIONS
);
