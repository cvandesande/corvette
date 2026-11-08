use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    Param,
    Model,
}

#[derive(Debug)]
pub enum Error {
    NoDevice,
    DeviceOutOfRange {
        index: i32,
        count: usize,
    },
    SoftwareDevice {
        index: i32,
        name: String,
    },
    NullHandle {
        what: &'static str,
    },
    InvalidPath,
    InvalidInputShape,
    Load {
        stage: LoadStage,
        path: PathBuf,
        code: i32,
    },
    NoInputBlob,
    NoOutputBlob,
    BlobName,
    VulkanNotEnabled,
    InputLength {
        expected: usize,
        actual: usize,
    },
    SetInput {
        blob: String,
        code: i32,
    },
    /// Reserved for a non-zero extract return: device loss is read from this variant.
    Extract {
        blob: String,
        code: i32,
    },
    MalformedOutput {
        reason: String,
    },
    NetsAlive {
        count: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => f.write_str("no Vulkan device enumerated"),
            Self::DeviceOutOfRange { index, count } => {
                write!(
                    f,
                    "device {index} out of range, {count} Vulkan device(s) present"
                )
            }
            Self::SoftwareDevice { index, name } => {
                write!(
                    f,
                    "device {index} ({name}) is a software (type=cpu) Vulkan device"
                )
            }
            Self::NullHandle { what } => write!(f, "{what} returned null"),
            Self::InvalidPath => {
                f.write_str("path cannot be passed to ncnn (non-UTF-8 or interior NUL)")
            }
            Self::InvalidInputShape => f.write_str(
                "input shape sides must be positive with a usize element count, and width*height must be a multiple of 4 because ncnn spaces the channels of an external Mat by the plane size rounded up to 16 bytes",
            ),
            Self::Load { stage, path, code } => match stage {
                LoadStage::Param => {
                    write!(
                        f,
                        "failed to parse parameter file {} ({code})",
                        path.display()
                    )
                }
                LoadStage::Model => {
                    write!(f, "failed to load weights {} ({code})", path.display())
                }
            },
            Self::NoInputBlob => f.write_str("the net declares no input blob"),
            Self::NoOutputBlob => f.write_str("the net declares no output blob"),
            Self::BlobName => f.write_str("blob name contains an interior NUL byte"),
            Self::VulkanNotEnabled => f.write_str("use_vulkan_compute did not stick"),
            Self::InputLength { expected, actual } => {
                write!(
                    f,
                    "input slice has {actual} values, the net expects {expected}"
                )
            }
            Self::SetInput { blob, code } => {
                write!(f, "set extractor input {blob:?}: ncnn returned {code}")
            }
            Self::Extract { blob, code } => {
                write!(f, "extract output {blob:?}: ncnn returned {code}")
            }
            Self::MalformedOutput { reason } => f.write_str(reason),
            Self::NetsAlive { count } => write!(f, "{count} net(s) still alive"),
        }
    }
}

impl std::error::Error for Error {}
