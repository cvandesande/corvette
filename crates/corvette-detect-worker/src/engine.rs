use std::fmt;

use corvette_detect_proto::{RAW_VALUES, TENSOR_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    pub reason: String,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for EngineError {}

pub trait Engine {
    /// # Errors
    /// Returns an `EngineError` when the model run fails or its output has the wrong length.
    fn infer(&mut self, rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError>;
}
