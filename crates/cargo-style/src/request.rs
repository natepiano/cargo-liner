//! The request that the reader reads from stdin, and its answer on stdout.

use std::collections::BTreeSet;
use std::io::BufWriter;
use std::io::Write;
use std::path::PathBuf;

use gix::ObjectId;
use serde::Deserialize;
use serde::Serialize;

use crate::blob_reader::BlobReader;
use crate::error::ReaderError;
use crate::module_sites::ModuleSitesQuery;

/// The facts that a request asks for.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FactKind {
    /// The number of lines that Python `str.splitlines` gives.
    Lines,
    /// Module declarations and their path attributes.
    ModuleScan,
    /// Declared items and their word spans.
    Declarations,
    /// Name offsets within each declared item body.
    NameSites,
    /// The words of the blob in the requested character class, including comments and literals.
    Names,
    /// The code tokens of the Python token reader.
    Tokens,
    /// The SHA-256 identity of code tokens and inner documentation comments.
    RustCode,
}

/// The character class used by a names search.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NameWordClass {
    /// The host POSIX alphanumeric class and underscore, under the caller's locale.
    #[default]
    PosixAlnum,
    /// ASCII letters, digits, and the underscore, as used by the C locale.
    Ascii,
}

/// Whether a request emits complete tree inventories.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(from = "bool")]
enum TreeOutput {
    /// Read only blob facts.
    #[default]
    Omit,
    /// Emit each tree inventory before the blob facts.
    Include,
}

impl From<bool> for TreeOutput {
    fn from(include: bool) -> Self { if include { Self::Include } else { Self::Omit } }
}

/// Read the requested blob facts and tree inventories.
#[derive(Debug, Deserialize)]
pub(crate) struct Request {
    repository:      PathBuf,
    commits:         Vec<String>,
    facts:           Vec<FactKind>,
    #[serde(default)]
    module_sites:    Vec<ModuleSitesQuery>,
    #[serde(default)]
    trees:           TreeOutput,
    #[serde(default = "default_extensions")]
    extensions:      Vec<String>,
    #[serde(default)]
    blobs:           Vec<String>,
    #[serde(default)]
    name_word_class: NameWordClass,
}

impl Request {
    /// Read one request from stdin.
    pub(crate) fn read() -> Result<Self, ReaderError> {
        serde_json::from_reader(std::io::stdin().lock()).map_err(ReaderError::Request)
    }

    /// Write the fact lines of every Rust blob at the commits to stdout.
    pub(crate) fn answer(self) -> Result<(), ReaderError> {
        let mut kinds = self.facts;
        kinds.sort_unstable();
        kinds.dedup();
        let reader = BlobReader::open(&self.repository)?;
        let inventories = reader.inventories(&self.commits)?;
        let mut blobs = BTreeSet::new();
        let suffixes: Vec<_> = self
            .extensions
            .iter()
            .map(|extension| format!(".{extension}").into_bytes())
            .collect();
        for inventory in &inventories {
            for (path, mode, kind, blob) in &inventory.entries {
                if !kinds.is_empty()
                    && kind == "blob"
                    && matches!(mode.as_str(), "100644" | "100755")
                    && suffixes.iter().any(|suffix| path.ends_with(suffix))
                {
                    blobs.insert(parse_blob(blob)?);
                }
            }
        }
        for blob in &self.blobs {
            blobs.insert(parse_blob(blob)?);
        }
        let blobs: Vec<_> = blobs.into_iter().collect();
        let mut output = BufWriter::new(std::io::stdout().lock());
        if self.trees == TreeOutput::Include {
            for inventory in &inventories {
                serde_json::to_writer(&mut output, inventory)
                    .map_err(ReaderError::TreeSerialize)?;
                output.write_all(b"\n").map_err(ReaderError::Write)?;
            }
        }
        reader.write_request(
            &blobs,
            &kinds,
            &self.module_sites,
            self.name_word_class,
            &mut output,
        )?;
        output.flush().map_err(ReaderError::Write)
    }
}

/// The source extension used by requests that omit the extension list.
fn default_extensions() -> Vec<String> { vec!["rs".to_owned()] }

fn parse_blob(blob: &str) -> Result<ObjectId, ReaderError> {
    ObjectId::from_hex(blob.as_bytes()).map_err(|source| ReaderError::QueryBlobId {
        blob: blob.to_owned(),
        source,
    })
}
