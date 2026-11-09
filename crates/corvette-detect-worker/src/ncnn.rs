use std::path::Path;

use corvette_detect_proto::{RAW_VALUES, TENSOR_BYTES};
use corvette_ncnn::{Device, InputShape, Net};

use crate::engine::{Engine, EngineError};

#[derive(Debug)]
pub struct NcnnEngine(Net);

impl NcnnEngine {
    /// # Errors
    /// Returns the net loader's error, or `InvalidInputShape` when a side does not fit `i32`.
    pub fn load(param: &Path, bin: &Path, device: Device) -> Result<Self, corvette_ncnn::Error> {
        let side = |value: usize| {
            i32::try_from(value).map_err(|_| corvette_ncnn::Error::InvalidInputShape)
        };
        let input = InputShape {
            width: side(corvette_yolo::WIDTH)?,
            height: side(corvette_yolo::HEIGHT)?,
            channels: side(corvette_yolo::CHANNELS)?,
        };
        Net::load(param, bin, device, input).map(Self)
    }
}

impl Engine for NcnnEngine {
    fn infer(&mut self, rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        let mut input = corvette_yolo::normalize(rgb);
        match self.0.infer(&mut input) {
            Ok(output) => {
                Box::try_from(output.data.into_boxed_slice()).map_err(|values: Box<[f32]>| {
                    EngineError {
                        reason: format!(
                            "output has {} values, expected {RAW_VALUES}",
                            values.len()
                        ),
                    }
                })
            }
            Err(error) => Err(EngineError {
                reason: error.to_string(),
            }),
        }
    }
}
