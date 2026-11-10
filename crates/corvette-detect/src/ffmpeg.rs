//! Builds the `ffmpeg` command that decodes one video input into raw detect frames.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decode {
    Cpu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Live { url: String },
    Segment { path: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecError {
    Size,
    Rate,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Size => f.write_str("frame size must be non-zero and even in both directions"),
            Self::Rate => f.write_str("frame rate must be non-zero"),
        }
    }
}

impl std::error::Error for SpecError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSpec {
    width: u32,
    height: u32,
    fps: u32,
    decode: Decode,
}

impl FrameSpec {
    /// # Errors
    /// Returns [`SpecError`] naming the first refused part: a size that is zero or odd, else a zero rate.
    pub const fn new(width: u32, height: u32, fps: u32, decode: Decode) -> Result<Self, SpecError> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(SpecError::Size);
        }
        if fps == 0 {
            return Err(SpecError::Rate);
        }
        Ok(Self {
            width,
            height,
            fps,
            decode,
        })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn fps(self) -> u32 {
        self.fps
    }
}

pub fn decode_command(
    program: impl AsRef<std::ffi::OsStr>,
    input: &Input,
    spec: FrameSpec,
) -> Command {
    let mut command = Command::new(program);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    match input {
        Input::Live { url } => command.args(["-rtsp_transport", "tcp"]).arg("-i").arg(url),
        Input::Segment { path } => command.arg("-i").arg(path),
    };
    match spec.decode {
        Decode::Cpu => {
            let filter = format_args!(
                "fps={},scale={}:{},eq=gamma=1.4:gamma_weight=0.5",
                spec.fps, spec.width, spec.height
            )
            .to_string();
            command
                .arg("-r")
                .arg(spec.fps.to_string())
                .arg("-vf")
                .arg(filter)
                .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "pipe:1"]);
        }
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command
}
