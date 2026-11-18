#![forbid(unsafe_code)]

pub mod annotate;
pub mod args;
pub mod compare;
pub mod compare_events;
pub mod export_events;
pub mod files;
pub mod latency;
pub mod motion_detect;
pub mod regions;
pub mod replay;
pub mod run;
pub mod tensors;

use std::io::Write;
use std::process::ExitCode;

const EXIT_MATCH: u8 = 0;
const EXIT_DIFFERENCE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_INPUT: u8 = 3;

const USAGE: &str = "\
usage: corvette-detect-tools COMMAND [flags]
  corvette-detect-tools tensors synthetic --out DIR --count N [--seed S]
  corvette-detect-tools tensors raw-rgb --in FILE --out DIR
  corvette-detect-tools run --socket PATH --in DIR --out DIR [--deadline-ms D]
  corvette-detect-tools compare --a DIR --b DIR [--tolerance T]
  corvette-detect-tools motion-detect --luma FILE --width W --height H --out FILE
    motion-detect optional: --threshold --contour-area --improve-contrast --lightning-threshold --frame-alpha --frame-height --mask
  corvette-detect-tools annotate --luma FILE --width W --height H --boxes FILE --frames LIST --out DIR
  corvette-detect-tools regions --boxes FILE --width W --height H --out FILE [--model-size S]
  corvette-detect-tools export-events --in FILE --out FILE [--camera NAME] [--label NAME]
  corvette-detect-tools compare-events --events FILE --record FILE [--record FILE ...] --out FILE [--zoned-only yes|no] [--min-seconds S]
  corvette-detect-tools replay --socket PATH --labels FILE --in SEGMENT --out FILE [--camera NAME] [--width W] [--height H] [--fps F] [--deadline-ms D] [--confirm X] [--area X,Y,...] [--area-name NAME] [--area-pad PX]
    replay optional: --camera --width --height --fps --deadline-ms --confirm --area --area-name --area-pad
flags are --name value pairs in any order
exit codes:
  0  success; for compare, the maximum absolute error is not above the tolerance
  1  compare only: the maximum absolute error is above the tolerance
  2  usage error
  3  input, output or format error
";

#[derive(Debug)]
pub enum Failure {
    Usage,
    Input(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(Failure::Usage) => {
            let mut stderr = std::io::stderr();
            let _ = stderr.write_all(USAGE.as_bytes());
            let _ = stderr.flush();
            ExitCode::from(EXIT_USAGE)
        }
        Err(Failure::Input(message)) => {
            let _ = writeln!(std::io::stderr(), "error: {message}");
            ExitCode::from(EXIT_INPUT)
        }
    }
}

fn dispatch(args: &[String]) -> Result<ExitCode, Failure> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(Failure::Usage);
    };
    let tail = args.get(1..).unwrap_or(&[]);
    match command {
        "tensors" => tensors::dispatch(tail),
        "run" => run::run(tail),
        "compare" => compare::run(tail),
        "motion-detect" => motion_detect::run(tail),
        "annotate" => annotate::run(tail),
        "regions" => regions::run(tail),
        "replay" => replay::run(tail),
        "export-events" => export_events::run(tail),
        "compare-events" => compare_events::run(tail),
        _ => Err(Failure::Usage),
    }
}
