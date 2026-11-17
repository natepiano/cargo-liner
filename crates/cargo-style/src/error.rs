//! The ways a read request fails.

use std::path::PathBuf;

use gix::ObjectId;

/// A failure that stops the whole request. A blob that is not UTF-8 is a fact reading, not an
/// error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReaderError {
    #[error("the request on stdin is not valid")]
    Request(#[source] serde_json::Error),
    #[error("cannot open the repository at {path}")]
    Open {
        path:   PathBuf,
        #[source]
        source: gix::Error,
    },
    #[error("`{commit}` is not a full commit id")]
    CommitId {
        commit: String,
        #[source]
        source: gix::Error,
    },
    #[error("`{blob}` is not a full query blob id")]
    QueryBlobId {
        blob:   String,
        #[source]
        source: gix::Error,
    },
    #[error("cannot read the tree of commit {commit}")]
    Tree {
        commit: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot read blob {blob}")]
    Blob {
        blob:   ObjectId,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot write the facts of blob {blob}")]
    Serialize {
        blob:   ObjectId,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot serialize the tree inventory")]
    TreeSerialize(#[source] serde_json::Error),
    #[error("cannot write the facts to stdout")]
    Write(#[source] std::io::Error),
}
