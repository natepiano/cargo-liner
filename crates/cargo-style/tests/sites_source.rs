//! Compare source sites with the pinned Python ledger.

#[path = "sites/parity.rs"]
mod parity;
mod support;

use parity::run_sites_parity;
use support::TestResult;

const ORACLE_SOURCE: &[u8] = include_bytes!("sites/source_oracle.py");

#[test]
fn source_sites_match_the_python_ledger() -> TestResult<()> {
    run_sites_parity(ORACLE_SOURCE, "cargo-style-source-sites-")
}
