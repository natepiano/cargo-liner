//! Named executables for tests that observe a process across exec.
//!
//! macOS uses a locally signed copy of `/bin/cat`: Homebrew framework Python
//! launchers exec `Python.app`, which changes the observed process name.
//! Other platforms use a copy of Python's interpreter: NixOS has no `/bin/cat`,
//! and its multicall cat cannot run under a caller-selected name.

use std::io;
#[cfg(not(target_os = "macos"))]
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// A copied executable that keeps its selected name and echoes stdin lines.
pub struct StdinEchoExecutable {
    path:      PathBuf,
    arguments: &'static [&'static str],
}

impl StdinEchoExecutable {
    /// Copy the platform's echo executable into `directory` under `name`.
    ///
    /// # Errors
    /// Returns an error if locating, copying, or signing the executable fails.
    pub fn copy_to(directory: &Path, name: &str) -> io::Result<Self> {
        let path = directory.join(name);
        #[cfg(target_os = "macos")]
        {
            std::fs::copy("/bin/cat", &path)?;
            if !Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(&path)
                .status()?
                .success()
            {
                return Err(io::Error::other("signing the census exec fixture failed"));
            }
            Ok(Self {
                path,
                arguments: &[],
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let python = Command::new("python3")
                .args(["-c", "import sys; print(sys.executable)"])
                .output()?;
            if !python.status.success() {
                return Err(io::Error::other(
                    "locating the census exec fixture's Python failed",
                ));
            }
            let source = String::from_utf8(python.stdout)
                .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
            std::fs::copy(source.trim(), &path)?;
            Ok(Self {
                path,
                arguments: &[
                    "-u",
                    "-c",
                    "import sys; [print(line.rstrip('\\n'),flush=True) for line in sys.stdin]",
                ],
            })
        }
    }

    /// The copied executable's path, suitable for exec's program and `argv[0]`.
    #[must_use]
    pub fn path(&self) -> &Path { &self.path }

    /// The payload arguments that make this executable echo stdin lines.
    #[must_use]
    pub const fn arguments(&self) -> &'static [&'static str] { self.arguments }
}
