//! The `cargo-mend` binary, without clippy's lints.

#![feature(rustc_private)]

// Links std through `librustc_driver`, as the library does; without it the
// binary links a second, static std.
extern crate rustc_driver;

use std::process::ExitCode;

fn main() -> ExitCode { cargo_mend::main(None) }
