//! Read Git objects when a replacement ref can change their contents.

use std::collections::BTreeSet;
use std::env;
use std::io;
use std::io::Seek as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use gix::ObjectId;
use gix::Repository;

pub(crate) enum ReplacementRefs {
    Listed(BTreeSet<ObjectId>),
    ResolveWithGit,
}

impl ReplacementRefs {
    pub(crate) fn read(repository: &Repository) -> Self {
        if env::var_os("GIT_REPLACE_REF_BASE").is_some() || env::var_os("GIT_NAMESPACE").is_some() {
            return Self::ResolveWithGit;
        }
        let Ok(refs) = repository.references() else {
            return Self::ResolveWithGit;
        };
        let Ok(replacements) = refs.prefixed("refs/replace/") else {
            return Self::ResolveWithGit;
        };
        let mut ids = BTreeSet::new();
        for reference in replacements {
            let Ok(reference) = reference else {
                return Self::ResolveWithGit;
            };
            if let Some(name) = reference.name().as_bstr().strip_prefix(b"refs/replace/")
                && let Ok(id) = ObjectId::from_hex(name)
            {
                let _ = ids.insert(id);
            }
        }
        Self::Listed(ids)
    }

    pub(crate) fn can_change_tree(&self) -> bool {
        match self {
            Self::Listed(ids) => !ids.is_empty(),
            Self::ResolveWithGit => true,
        }
    }

    pub(crate) fn can_change_blob(&self, blob: ObjectId) -> bool {
        match self {
            Self::Listed(ids) => ids.contains(&blob),
            Self::ResolveWithGit => true,
        }
    }
}

pub(crate) fn read(repo: &Path, arguments: &[&str]) -> io::Result<Vec<u8>> {
    let output = command(repo).args(arguments).output()?;
    contents(output)
}

pub(crate) fn blob(repo: &Path, id: ObjectId) -> io::Result<Option<Vec<u8>>> {
    let mut input = tempfile::tempfile()?;
    writeln!(input, "{id}")?;
    input.rewind()?;
    let output = command(repo)
        .args(["cat-file", "--batch"])
        .stdin(input)
        .output()?;
    let reply = contents(output)?;
    let mut lines = reply.splitn(2, |&byte| byte == b'\n');
    let header = std::str::from_utf8(lines.next().unwrap_or_default()).map_err(io::Error::other)?;
    let mut fields = header.split(' ');
    let reported = fields.next();
    let kind = fields.next();
    let size = fields.next();
    if reported == Some(id.to_string().as_str()) && fields.next().is_none() {
        if kind == Some("missing") && size.is_none() {
            return Ok(None);
        }
        if kind == Some("blob")
            && let Some(size) = size.and_then(|size| size.parse::<usize>().ok())
            && let Some(bytes) = lines.next().and_then(|line| line.strip_suffix(b"\n"))
            && bytes.len() == size
        {
            return Ok(Some(bytes.to_vec()));
        }
    }
    Err(io::Error::other(format!(
        "unexpected git cat-file response: {header}"
    )))
}

fn command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo);
    command
}

fn contents(output: Output) -> io::Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(io::Error::other(
        String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    ))
}
