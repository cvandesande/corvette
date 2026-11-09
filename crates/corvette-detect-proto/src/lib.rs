//! Byte layout, types and codecs for the frames exchanged with the Corvette inference worker.
#![forbid(unsafe_code)]

pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 6;
pub const ID_LEN: usize = 8;
pub const DEADLINE_LEN: usize = 4;
// Sizes restate corvette-yolo's: 640 x 640 x 3 input bytes, 50 x 8,400 raw output values.
pub const TENSOR_BYTES: usize = 640 * 640 * 3;
pub const RAW_VALUES: usize = 50 * 8_400;
pub const MAX_ROWS: usize = 20;
pub const ROW_LEN: usize = 24;
pub const MAX_BODY: usize = ID_LEN + RAW_VALUES * 4;

mod header;
mod reply;
mod request;

pub use header::{DecodeError, Header, HeaderError, Kind, parse_header};
pub use reply::{EncodeError, ErrorCode, Reply, Row};
pub use request::Request;
