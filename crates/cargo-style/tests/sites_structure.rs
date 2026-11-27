//! Compare structure sites with the pinned Python ledger.
//! Cover workspace member patterns, absolute paths, and TOML 1.0 parsing.

#[path = "sites/parity.rs"]
mod parity;
mod support;

use parity::run_sites_parity;
use support::TestResult;

const ORACLE_SOURCE: &[u8] = include_bytes!("sites/structure_oracle.py");

#[test]
fn structure_sites_match_the_python_ledger() -> TestResult<()> {
    run_sites_parity(ORACLE_SOURCE, "cargo-style-structure-sites-")
}
