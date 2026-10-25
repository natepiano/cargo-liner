//! Bounded discovery of the installation configuration gix must read.

use std::ffi::OsString;
use std::io;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::OnceLock;

use gix::open::Options;

use super::child::ChildProcess;
use super::command;

static INSTALLATION_CONFIGURATION: OnceLock<GitInstallationConfiguration> = OnceLock::new();

enum GitInstallationConfiguration {
    NoFile,
    File {
        path:  PathBuf,
        scope: InstallationScope,
    },
}

enum InstallationScope {
    System,
    Other,
}

/// Supply the same installation file gix discovers, without its unbounded subprocess.
pub(super) fn open_options() -> io::Result<Options> {
    command::with_spawn_lock(|| {
        if INSTALLATION_CONFIGURATION.get().is_none() {
            let configuration = discover()?;
            let _ = INSTALLATION_CONFIGURATION.set(configuration);
        }
        let configuration = INSTALLATION_CONFIGURATION
            .get()
            .ok_or_else(|| io::Error::other("installation configuration was not discovered"))?;
        let mut permissions = gix::open::Permissions::default();
        let mut options = gix::open::Options::default().strict_config(true);
        if let GitInstallationConfiguration::File { path, scope } = configuration {
            permissions.config.git_binary = true;
            let path = match scope {
                InstallationScope::System => std::env::var_os("GIT_CONFIG_SYSTEM")
                    .map_or_else(|| path.clone(), PathBuf::from),
                InstallationScope::Other => path.clone(),
            };
            options = options.git_installation_config_path(path);
        }
        Ok(options.permissions(permissions))
    })?
}

fn discover() -> io::Result<GitInstallationConfiguration> {
    let output = match probe(true) {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(GitInstallationConfiguration::NoFile);
        },
        Err(error) => return Err(error),
    };
    if output.status.success() {
        let mut fields = output.stdout.split(|byte| *byte == 0);
        while let (Some(scope), Some(origin), Some(_)) =
            (fields.next(), fields.next(), fields.next())
        {
            if let Some(path) = origin.strip_prefix(b"file:") {
                return Ok(GitInstallationConfiguration::File {
                    path:  PathBuf::from(OsString::from_vec(path.to_vec())),
                    scope: if scope == b"system" {
                        InstallationScope::System
                    } else {
                        InstallationScope::Other
                    },
                });
            }
        }
        return Ok(GitInstallationConfiguration::NoFile);
    }
    // Git before 2.26 does not report scopes; gix also retries without that flag.
    let output = probe(false)?;
    let first = output
        .stdout
        .split(|byte| *byte == 0)
        .next()
        .unwrap_or_default();
    Ok(first
        .strip_prefix(b"file:")
        .map_or(GitInstallationConfiguration::NoFile, |path| {
            GitInstallationConfiguration::File {
                path:  PathBuf::from(OsString::from_vec(path.to_vec())),
                scope: InstallationScope::Other,
            }
        }))
}

fn probe(show_scope: bool) -> io::Result<Output> {
    let mut command = Command::new("git");
    command.args(["config", "-lz", "--show-origin", "--no-includes"]);
    if show_scope {
        command.arg("--show-scope");
    }
    command.arg("--name-only").current_dir("/");
    for variable in [
        "GIT_CONFIG",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(variable);
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_DIR", "/dev/null")
        .env("GIT_WORK_TREE", "/dev/null")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // open_options owns the spawn lock, including this dependency-replacement probe.
    ChildProcess::spawn(&mut command)?.output(
        Vec::new(),
        super::protocol::CONTROL_REPLY_DEADLINE,
        |_| {},
    )
}
