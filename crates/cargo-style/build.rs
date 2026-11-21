//! Set the reader version from the sources that define blob facts.

use std::error::Error;
use std::fmt::Write;
use std::fs;

use sha2::Digest;
use sha2::Sha256;

const FACT_SOURCES: &[&str] = &[
    "src/blob_reader.rs",
    "src/constants.rs",
    "src/declarations.rs",
    "src/facts.rs",
    "src/module_scan.rs",
    "src/module_sites.rs",
    "src/name_sites.rs",
    "src/tokenizer.rs",
    "src/unicode_tables.rs",
    "src/word_syntax.rs",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut digest = Sha256::new();
    for source in FACT_SOURCES {
        println!("cargo:rerun-if-changed={source}");
        let bytes = fs::read(source)?;
        digest.update(source.as_bytes());
        digest.update([0]);
        digest.update(u64::try_from(bytes.len())?.to_le_bytes());
        digest.update(bytes);
    }
    let mut version = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(version, "{byte:02x}")?;
    }
    println!("cargo:rustc-env=CARGO_STYLE_READER_VERSION={version}");
    Ok(())
}
