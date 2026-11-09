use std::fs::{self, File};
use std::io::Read as _;
use std::path::Path;
use std::process::ExitCode;

use corvette_detect_proto::TENSOR_BYTES;

use crate::{EXIT_MATCH, Failure, args, files};

const SYNTHETIC_FLAGS: &[&str] = &["out", "count", "seed"];
const RAW_RGB_FLAGS: &[&str] = &["in", "out"];
const MAX_COUNT: u64 = 99_999;
const MIX_GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
const MIX_MULTIPLIER_A: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_MULTIPLIER_B: u64 = 0x94D0_49BB_1331_11EB;
const BYTES_PER_OUTPUT: usize = 8;

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(state: u64) -> Self {
        Self { state }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(MIX_GOLDEN);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(MIX_MULTIPLIER_A);
        z = (z ^ (z >> 27)).wrapping_mul(MIX_MULTIPLIER_B);
        z ^ (z >> 31)
    }

    fn fill(&mut self, buffer: &mut [u8]) {
        for chunk in buffer.chunks_exact_mut(BYTES_PER_OUTPUT) {
            for (destination, source) in chunk.iter_mut().zip(self.next_u64().to_le_bytes()) {
                *destination = source;
            }
        }
    }
}

pub(crate) fn dispatch(args: &[String]) -> Result<ExitCode, Failure> {
    match args.first().map(String::as_str) {
        Some("synthetic") => synthetic(args.get(1..).unwrap_or(&[])),
        Some("raw-rgb") => raw_rgb(args.get(1..).unwrap_or(&[])),
        _ => Err(Failure::Usage),
    }
}

fn synthetic(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = args::Flags::parse(args, SYNTHETIC_FLAGS).ok_or(Failure::Usage)?;
    let out = flags.required("out")?;
    let count = parse_count(flags.required("count")?)?;
    let seed = flags.get("seed").map_or(Ok(0), |text| {
        text.parse::<u64>().map_err(|_| Failure::Usage)
    })?;
    if files::holds_tensor_files(out)? {
        return Err(Failure::Input(format!("{out} already holds tensor files")));
    }
    fs::create_dir_all(out)
        .map_err(|error| Failure::Input(format!("cannot create {out}: {error}")))?;
    let mut generator = SplitMix64::new(seed);
    let mut buffer = vec![0u8; TENSOR_BYTES];
    for index in 0..count {
        generator.fill(&mut buffer);
        let name = files::tensor_name(index);
        fs::write(Path::new(out).join(&name), &buffer).map_err(|error| {
            Failure::Input(format!("cannot write tensor {name} into {out}: {error}"))
        })?;
    }
    println!("wrote {count} tensors to {out}");
    Ok(ExitCode::from(EXIT_MATCH))
}

fn raw_rgb(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = args::Flags::parse(args, RAW_RGB_FLAGS).ok_or(Failure::Usage)?;
    let input = flags.required("in")?;
    let out = flags.required("out")?;
    let tensor_bytes = u64::try_from(TENSOR_BYTES)
        .map_err(|_| Failure::Input(String::from("tensor size too large")))?;
    let length = fs::metadata(input)
        .map(|meta| meta.len())
        .map_err(|error| Failure::Input(format!("cannot read {input}: {error}")))?;
    if length == 0 || length % tensor_bytes != 0 {
        return Err(Failure::Input(format!(
            "{input} is {length} bytes, expected a positive multiple of {tensor_bytes}"
        )));
    }
    if files::holds_tensor_files(out)? {
        return Err(Failure::Input(format!("{out} already holds tensor files")));
    }
    fs::create_dir_all(out)
        .map_err(|error| Failure::Input(format!("cannot create {out}: {error}")))?;
    let mut source = File::open(input)
        .map_err(|error| Failure::Input(format!("cannot read {input}: {error}")))?;
    let count = length / tensor_bytes;
    for index in 0..count {
        let name = files::tensor_name(index);
        let mut sink = File::create(Path::new(out).join(&name)).map_err(|error| {
            Failure::Input(format!("cannot write tensor {name} into {out}: {error}"))
        })?;
        let copied = std::io::copy(&mut source.by_ref().take(tensor_bytes), &mut sink)
            .map_err(|error| Failure::Input(format!("cannot copy {input}: {error}")))?;
        if copied != tensor_bytes {
            return Err(Failure::Input(format!(
                "{input} is shorter than its recorded length"
            )));
        }
    }
    println!("wrote {count} tensors to {out}");
    Ok(ExitCode::from(EXIT_MATCH))
}

fn parse_count(text: &str) -> Result<u64, Failure> {
    text.parse::<u64>()
        .ok()
        .filter(|count| (1..=MAX_COUNT).contains(count))
        .ok_or(Failure::Usage)
}
