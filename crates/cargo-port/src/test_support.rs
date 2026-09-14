//! Shared helpers for unit tests.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use tokio::runtime::Runtime;

pub(crate) fn normalize_line_endings(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    normalized.trim_end_matches(['\r', '\n']).to_string()
}

/// Process-wide tokio runtime for tests that need a `Handle` or
/// `block_on`. Created once on first use and shared thereafter so each
/// test isn't paying for runtime startup.
pub(crate) fn test_runtime() -> &'static Runtime {
    static TEST_RT: OnceLock<Runtime> = OnceLock::new();
    TEST_RT.get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime starts"))
}

/// Build a `HeaderMap` from `(name, value)` pairs. Panics on invalid
/// header names or values — tests should fail loudly on a typo.
pub(crate) fn header_map(entries: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let name: HeaderName = (*name).parse().expect("test header name is valid");
        let value = HeaderValue::from_str(value).expect("test header value is valid");
        headers.insert(name, value);
    }
    headers
}

/// Initialize a repository with one commit, including any existing fixture files.
pub(crate) fn init_git_repo(dir: &Path) {
    let commands: [&[&str]; 3] = [
        &["init"],
        &["add", "."],
        &[
            "-c",
            "user.name=cargo-port-tests",
            "-c",
            "user.email=cargo-port-tests@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    ];
    for args in commands {
        let output = Command::new(git_binary())
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

pub(crate) fn git_binary() -> &'static str {
    if Path::new("/usr/bin/git").is_file() {
        "/usr/bin/git"
    } else {
        "git"
    }
}
