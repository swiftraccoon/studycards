//! The `studycards` program; see [`studycards::cli`] for its command line.

use std::env;
use std::process::ExitCode;

use studycards::cli;

fn main() -> ExitCode {
    cli::main(env::args().skip(1))
}
