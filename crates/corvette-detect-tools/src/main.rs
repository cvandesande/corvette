#![forbid(unsafe_code)]

pub mod args;
pub mod compare;
pub mod files;
pub mod latency;
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
  corvette-detect-tools compare --a DIR --b DIR [--tolerance T]
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
        "compare" => compare::run(tail),
        _ => Err(Failure::Usage),
    }
}
