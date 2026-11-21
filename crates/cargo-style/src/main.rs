//! `cargo-style` reads facts from Rust blobs in Git commits for the style refill.
//!
//! It reads one JSON request on stdin: `{"repository": <path>, "commits": [<full ids>], "facts":
//! [<kinds>]}`. It writes one JSON line on stdout for each blob and fact kind, in blob id order:
//! `{"blob": <id>, "fact": <kind>, "value": <value>}`, or `"unreadable": "not_utf8"` in place of
//! the value. Each fact matches the Python reader in `scripts/style` that it ports.
//! `RAYON_NUM_THREADS` sets the thread count; the default is one thread for each CPU.

mod blob_reader;
mod cache_constants;
mod constants;
mod declarations;
mod error;
mod fact_store;
mod facts;
mod finding;
mod finding_anchor;
mod finding_source;
mod ledger_read;
mod module_scan;
mod module_sites;
mod name_sites;
mod request;
mod source_lines;
mod sqlite_read;
mod state_directory;
mod tokenizer;
mod tree_inventory;
mod unicode_tables;
mod word_syntax;

use std::error::Error;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;
use std::vec::IntoIter;

use libc::LC_CTYPE;

use crate::constants::PROTOCOL_VERSION;
use crate::constants::STYLE_SUBCOMMAND;
use crate::constants::VERSION_FLAG;
use crate::finding::FINDING_SUBCOMMAND;
use crate::finding::FindingRequest;
use crate::request::Request;

/// The operation selected by a direct call or the Cargo subcommand.
enum ReaderCommand {
    Facts,
    Finding(IntoIter<OsString>),
    Version,
}

impl From<Vec<OsString>> for ReaderCommand {
    fn from(arguments: Vec<OsString>) -> Self {
        let mut arguments = arguments.into_iter();
        let first = arguments.next();
        let operation = if first.as_deref() == Some(OsStr::new(STYLE_SUBCOMMAND)) {
            arguments.next()
        } else {
            first
        };
        match operation.as_deref() {
            Some(argument) if argument == OsStr::new(VERSION_FLAG) => Self::Version,
            Some(argument) if argument == OsStr::new(FINDING_SUBCOMMAND) => {
                Self::Finding(arguments)
            },
            _ => Self::Facts,
        }
    }
}

#[allow(
    unsafe_code,
    reason = "The process selects the host locale before it starts reader threads."
)]
fn main() -> ExitCode {
    match ReaderCommand::from(std::env::args_os().skip(1).collect::<Vec<_>>()) {
        ReaderCommand::Version => {
            println!(
                "cargo-style {} protocol {PROTOCOL_VERSION}",
                env!("CARGO_PKG_VERSION")
            );
            return ExitCode::SUCCESS;
        },
        ReaderCommand::Finding(arguments) => {
            return match FindingRequest::parse(arguments).and_then(FindingRequest::answer) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!(
                        "cargo-style: {}",
                        error_chain(&error).replace(['\r', '\n'], " ")
                    );
                    ExitCode::FAILURE
                },
            };
        },
        ReaderCommand::Facts => {},
    }

    // SAFETY: The locale changes before the reader starts any worker threads.
    unsafe {
        libc::setlocale(LC_CTYPE, c"".as_ptr());
    }

    match Request::read().and_then(Request::answer) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cargo-style: {}", error_chain(&error));
            ExitCode::FAILURE
        },
    }
}

fn error_chain(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&cause| cause.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}
