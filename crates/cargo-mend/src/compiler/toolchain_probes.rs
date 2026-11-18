//! Reuse successful compiler queries across mend runs.
//!
//! Cargo asks each new target directory for the compiler version and target
//! facts. Each answer starts the mend wrapper and rustc, and each of those
//! loads the compiler library. The answers depend only on the compiler file,
//! the query arguments and a few toolchain variables, so mend keeps them in
//! the shared wrapper cache of its build. A changed compiler file changes the
//! key, and a new mend build starts a new cache.

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
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use tempfile::TempDir;

use super::constants::TOOLCHAIN_PROBES_ENV;
use super::constants::WRAPPER_CACHE_TOOLCHAIN_QUERIES_DIRECTORY;

/// Owns compiler-query results for every analysis pass of a CLI invocation.
pub(crate) enum ToolchainProbes {
    /// The shared wrapper cache keeps results for later runs of this mend build.
    Retained(PathBuf),
    /// A run without the shared cache keeps results until it ends.
    RunLocal(TempDir),
}

impl ToolchainProbes {
    pub(super) fn retained(wrapper_cache: &Path) -> Result<Self> {
        let directory = wrapper_cache.join(WRAPPER_CACHE_TOOLCHAIN_QUERIES_DIRECTORY);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "create mend toolchain probe directory {}",
                directory.display()
            )
        })?;
        Ok(Self::Retained(directory))
    }

    pub(super) fn run_local() -> Result<Self> {
        tempfile::tempdir()
            .map(Self::RunLocal)
            .context("create mend toolchain probe directory")
    }

    fn directory(&self) -> &Path {
        match self {
            Self::Retained(directory) => directory,
            Self::RunLocal(directory) => directory.path(),
        }
    }

    pub(super) fn configure(&self, command: &mut Command) {
        command.env(TOOLCHAIN_PROBES_ENV, self.directory());
    }
}

pub(super) enum ProbeInvocation {
    Compiler,
    Query(ExitCode),
}

/// The key of one reusable query. The working directory is not part of it:
/// Cargo's queries name no relative input, and each project runs them from
/// its own directory.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct ProbeRequest {
    compiler:    PathBuf,
    identity:    CompilerIdentity,
    arguments:   Vec<OsString>,
    environment: Vec<(String, OsString)>,
}

/// A new compiler file at the same path has a new size or modified time.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct CompilerIdentity {
    length:   u64,
    modified: SystemTime,
}

#[derive(Serialize, Deserialize)]
struct ToolchainQueryResult {
    request:   ProbeRequest,
    stdout:    Vec<u8>,
    stderr:    Vec<u8>,
    exit_code: u8,
}

/// Only Cargo's input-free queries are reusable. Source compilations and
/// ambient wrappers keep their normal execution path. A compiler that is not
/// a readable file has no identity, so its queries also keep that path.
pub(super) fn run_query(arguments: &[OsString]) -> Result<ProbeInvocation> {
    let Some(directory) = env::var_os(TOOLCHAIN_PROBES_ENV) else {
        return Ok(ProbeInvocation::Compiler);
    };
    if !is_query(arguments) {
        return Ok(ProbeInvocation::Compiler);
    }
    let Ok(metadata) = fs::metadata(&arguments[1]) else {
        return Ok(ProbeInvocation::Compiler);
    };
    let request = ProbeRequest {
        compiler:    PathBuf::from(&arguments[1]),
        identity:    CompilerIdentity {
            length:   metadata.len(),
            modified: metadata.modified()?,
        },
        arguments:   arguments[2..].to_vec(),
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
    let probe = read_or_run(Path::new(&directory), request, &env::current_dir()?)?;
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

fn read_or_run(
    directory: &Path,
    request: ProbeRequest,
    working_directory: &Path,
) -> Result<ToolchainQueryResult> {
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
    let output = Command::new(&request.compiler)
        .args(&request.arguments)
        .current_dir(working_directory)
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
    use std::path::Path;

    #[cfg(unix)]
    use super::CompilerIdentity;
    #[cfg(unix)]
    use super::ProbeRequest;
    #[cfg(unix)]
    use super::ToolchainProbes;
    use super::is_query;
    #[cfg(unix)]
    use super::read_or_run;

    #[cfg(unix)]
    fn request(compiler: &Path, argument: &str) -> anyhow::Result<ProbeRequest> {
        let metadata = fs::metadata(compiler)?;
        Ok(ProbeRequest {
            compiler:    compiler.to_path_buf(),
            identity:    CompilerIdentity {
                length:   metadata.len(),
                modified: metadata.modified()?,
            },
            arguments:   vec![argument.into()],
            environment: Vec::new(),
        })
    }

    #[cfg(unix)]
    #[test]
    fn queries_are_shared_across_directories_until_the_compiler_or_cache_changes()
    -> anyhow::Result<()> {
        let fixture = tempfile::tempdir()?;
        let compiler = fixture.path().join("compiler");
        let counter = fixture.path().join("starts");
        let script = format!(
            "#!/bin/sh\nprintf x >> '{}'\nprintf '%s' \"$1\"\n",
            counter.display()
        );
        fs::write(&compiler, &script)?;
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700))?;
        let cache = ToolchainProbes::run_local()?;
        let project = fixture.path();
        let other_project = tempfile::tempdir()?;
        let first = read_or_run(cache.directory(), request(&compiler, "-vV")?, project)?;
        let second = read_or_run(cache.directory(), request(&compiler, "-vV")?, project)?;
        assert_eq!(first.stdout, b"-vV");
        assert_eq!(second.stdout, first.stdout);
        assert_eq!(fs::read(&counter)?, b"x");
        let other_directory = read_or_run(
            cache.directory(),
            request(&compiler, "-vV")?,
            other_project.path(),
        )?;
        assert_eq!(other_directory.stdout, first.stdout);
        assert_eq!(fs::read(&counter)?, b"x");
        let different = read_or_run(
            cache.directory(),
            request(&compiler, "--print=cfg")?,
            project,
        )?;
        assert_eq!(different.stdout, b"--print=cfg");
        assert_eq!(fs::read(&counter)?, b"xx");
        fs::write(&compiler, format!("{script}# updated compiler\n"))?;
        read_or_run(cache.directory(), request(&compiler, "-vV")?, project)?;
        assert_eq!(fs::read(&counter)?, b"xxx");
        let another_cache = ToolchainProbes::run_local()?;
        read_or_run(
            another_cache.directory(),
            request(&compiler, "-vV")?,
            project,
        )?;
        assert_eq!(fs::read(counter)?, b"xxxx");
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
