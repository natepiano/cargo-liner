//! Reuse successful compiler queries within one mend invocation.
//!
//! Cargo can repeat the same toolchain query as fix passes change its wrapper
//! fingerprint. A private directory belongs to the CLI, not to a compilation
//! unit or a persistent fixture cache, and disappears when that invocation ends.

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use tempfile::TempDir;

const PROBE_DIRECTORY_ENV: &str = "MEND_TOOLCHAIN_PROBES";

/// Owns compiler-query results for every analysis pass of one CLI invocation.
pub(crate) struct ToolchainProbes(TempDir);

impl ToolchainProbes {
    pub(crate) fn new() -> Result<Self> {
        tempfile::tempdir()
            .map(Self)
            .context("create mend toolchain probe directory")
    }

    pub(super) fn configure(&self, command: &mut Command) {
        command.env(PROBE_DIRECTORY_ENV, self.0.path());
    }
}

pub(super) enum ProbeInvocation {
    Compiler,
    Query(ExitCode),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct ProbeRequest {
    arguments:   Vec<OsString>,
    directory:   PathBuf,
    environment: Vec<(String, OsString)>,
}

#[derive(Serialize, Deserialize)]
struct ToolchainQueryResult {
    request:   ProbeRequest,
    stdout:    Vec<u8>,
    stderr:    Vec<u8>,
    exit_code: u8,
}

/// Only Cargo's input-free queries are reusable. Source compilations and
/// ambient wrappers keep their normal execution path.
pub(super) fn run_query(arguments: &[OsString]) -> Result<ProbeInvocation> {
    let Some(directory) = env::var_os(PROBE_DIRECTORY_ENV) else {
        return Ok(ProbeInvocation::Compiler);
    };
    if !is_query(arguments) {
        return Ok(ProbeInvocation::Compiler);
    }
    let request = ProbeRequest {
        arguments:   arguments.to_vec(),
        directory:   env::current_dir()?,
        environment: [
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "RUSTC_BOOTSTRAP",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
        ]
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|value| (name.to_string(), value)))
        .collect(),
    };
    let probe = read_or_run(Path::new(&directory), request)?;
    io::stdout().write_all(&probe.stdout)?;
    io::stderr().write_all(&probe.stderr)?;
    Ok(ProbeInvocation::Query(ExitCode::from(probe.exit_code)))
}

fn is_query(arguments: &[OsString]) -> bool {
    let arguments = &arguments[2..];
    arguments == ["-vV"]
        || (arguments.first().is_some_and(|arg| arg == "-")
            && arguments
                .windows(2)
                .any(|pair| pair == ["--crate-name", "___"])
            && arguments.iter().any(|arg| arg == "--print=file-names")
            && !arguments
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--emit")))
}

fn read_or_run(directory: &Path, request: ProbeRequest) -> Result<ToolchainQueryResult> {
    let mut hash = DefaultHasher::new();
    request.hash(&mut hash);
    let path = directory.join(format!("{:016x}.json", hash.finish()));
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    if let Ok(bytes) = fs::read(&path)
        && let Ok(probe) = serde_json::from_slice::<ToolchainQueryResult>(&bytes)
        && probe.request == request
        && probe.exit_code == 0
    {
        return Ok(probe);
    }
    let output = Command::new(&request.arguments[1])
        .args(&request.arguments[2..])
        .current_dir(&request.directory)
        .stdin(Stdio::null())
        .output()
        .context("query rustc toolchain")?;
    let exit_code = output
        .status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1);
    let probe = ToolchainQueryResult {
        request,
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code,
    };
    if output.status.success() {
        fs::write(path, serde_json::to_vec(&probe)?)?;
    }
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    use super::ProbeRequest;
    #[cfg(unix)]
    use super::ToolchainProbes;
    use super::is_query;
    #[cfg(unix)]
    use super::read_or_run;

    #[cfg(unix)]
    #[test]
    fn identical_queries_share_one_child_but_different_queries_and_runs_do_not()
    -> anyhow::Result<()> {
        let fixture = tempfile::tempdir()?;
        let compiler = fixture.path().join("compiler");
        let counter = fixture.path().join("starts");
        fs::write(
            &compiler,
            "#!/bin/sh\nprintf x >> starts\nprintf '%s' \"$1\"\n",
        )?;
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700))?;
        let request = ProbeRequest {
            arguments:   vec!["mend".into(), compiler.into_os_string(), "-vV".into()],
            directory:   fixture.path().to_path_buf(),
            environment: Vec::new(),
        };
        let session = ToolchainProbes::new()?;
        let first = read_or_run(session.0.path(), request.clone())?;
        let second = read_or_run(session.0.path(), request.clone())?;
        assert_eq!(first.stdout, b"-vV");
        assert_eq!(second.stdout, first.stdout);
        assert_eq!(fs::read(&counter)?, b"x");
        let mut different = request.clone();
        different.arguments[2] = "--print=cfg".into();
        let third = read_or_run(session.0.path(), different)?;
        assert_eq!(third.stdout, b"--print=cfg");
        assert_eq!(fs::read(&counter)?, b"xx");
        let another_session = ToolchainProbes::new()?;
        read_or_run(another_session.0.path(), request)?;
        assert_eq!(fs::read(counter)?, b"xxx");
        Ok(())
    }

    #[test]
    fn compiler_sources_are_never_reused_as_toolchain_queries() {
        for (args, query) in [
            (vec!["-vV"], true),
            (vec!["-", "--crate-name", "___", "--print=file-names"], true),
            (
                vec!["source.rs", "--crate-name", "___", "--print=file-names"],
                false,
            ),
            (
                vec![
                    "-",
                    "--crate-name",
                    "___",
                    "--print=file-names",
                    "--emit=metadata",
                ],
                false,
            ),
            (vec!["lib.rs", "--emit=metadata"], false),
        ] {
            let mut invocation = vec![OsString::from("mend"), OsString::from("rustc")];
            invocation.extend(args.into_iter().map(OsString::from));
            assert_eq!(is_query(&invocation), query);
        }
    }
}
