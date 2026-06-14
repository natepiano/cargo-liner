use std::io;
use std::io::ErrorKind;
use std::process::Output;

/// Result of the one-shot `gh auth token` probe run at startup. Holds the
/// token when authenticated; otherwise records *why* there is no token so
/// the UI can give the right remediation — install `gh` versus run `gh
/// auth login`. Keeping token and reason in one enum makes the "token
/// present but `gh` missing" combination unrepresentable.
#[derive(Clone)]
pub(super) enum GithubAuth {
    Authenticated(String),
    /// `gh` ran but returned no token (the user is not logged in).
    Unauthenticated,
    /// The `gh` binary was not found on `PATH`.
    NotInstalled,
}

impl GithubAuth {
    /// Classify the outcome of the startup `gh auth token` probe. A
    /// success exit yields the trimmed token — or `Unauthenticated` when
    /// stdout is not valid UTF-8. A spawn error of kind `NotFound` means
    /// the `gh` binary is absent; every other outcome (non-success exit,
    /// other spawn errors) is treated as logged-out.
    pub(super) fn classify(output: io::Result<Output>) -> Self {
        match output {
            Ok(output) if output.status.success() => String::from_utf8(output.stdout)
                .map_or(Self::Unauthenticated, |token| {
                    Self::Authenticated(token.trim().to_string())
                }),
            Err(error) if error.kind() == ErrorKind::NotFound => Self::NotInstalled,
            Ok(_) | Err(_) => Self::Unauthenticated,
        }
    }

    /// The bearer token when authenticated; `None` for either gap.
    pub(super) const fn token(&self) -> Option<&str> {
        match self {
            Self::Authenticated(token) => Some(token.as_str()),
            Self::Unauthenticated | Self::NotInstalled => None,
        }
    }

    /// Projects the auth state to the gap the UI surfaces, dropping the
    /// token. `None` means authenticated — there is nothing to warn about.
    pub(super) const fn gap(&self) -> Option<GithubAuthGap> {
        match self {
            Self::Authenticated(_) => None,
            Self::Unauthenticated => Some(GithubAuthGap::Unauthenticated),
            Self::NotInstalled => Some(GithubAuthGap::NotInstalled),
        }
    }
}

/// Why GitHub calls are disabled, surfaced to the UI so the startup toast
/// and git-pane row give the right remediation. Excludes the authenticated
/// case — there is no gap to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GithubAuthGap {
    /// The `gh` binary was not found on `PATH`.
    NotInstalled,
    /// `gh` is installed but returned no token.
    Unauthenticated,
}

#[cfg(test)]
/// Exercises `GithubAuth::classify` directly with constructed process
/// outcomes — the one place the missing-vs-logged-out distinction is
/// decided. Gated to unix because `ExitStatus` is only constructible
/// there (`ExitStatusExt::from_raw`); the primary platforms are unix.
#[cfg(unix)]
mod classify {
    use std::io;
    use std::io::ErrorKind;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::process::Output;

    use super::GithubAuth;

    fn gh_output(raw_wait_status: i32, stdout: &[u8]) -> Output {
        Output {
            status: ExitStatus::from_raw(raw_wait_status),
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn success_exit_with_token_is_authenticated() {
        // raw wait status 0 encodes a normal exit with code 0 (success).
        let github_auth = GithubAuth::classify(Ok(gh_output(0, b"  gho_abc123\n")));
        assert!(matches!(github_auth, GithubAuth::Authenticated(token) if token == "gho_abc123"));
    }

    #[test]
    fn success_exit_with_invalid_utf8_is_unauthenticated() {
        let github_auth = GithubAuth::classify(Ok(gh_output(0, &[0xff, 0xfe])));
        assert!(matches!(github_auth, GithubAuth::Unauthenticated));
    }

    #[test]
    fn nonsuccess_exit_is_unauthenticated() {
        // raw wait status `1 << 8` encodes a normal exit with code 1.
        let github_auth = GithubAuth::classify(Ok(gh_output(1 << 8, b"not logged in")));
        assert!(matches!(github_auth, GithubAuth::Unauthenticated));
    }

    #[test]
    fn missing_binary_is_not_installed() {
        let github_auth = GithubAuth::classify(Err(io::Error::from(ErrorKind::NotFound)));
        assert!(matches!(github_auth, GithubAuth::NotInstalled));
    }

    #[test]
    fn other_spawn_error_is_unauthenticated() {
        let github_auth = GithubAuth::classify(Err(io::Error::from(ErrorKind::PermissionDenied)));
        assert!(matches!(github_auth, GithubAuth::Unauthenticated));
    }
}
