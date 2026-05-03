//! Shared helpers for unit tests.

use std::sync::OnceLock;

use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;

pub(crate) fn normalize_line_endings(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    normalized.trim_end_matches(['\r', '\n']).to_string()
}

/// Process-wide tokio runtime for tests that need a `Handle` or
/// `block_on`. Created once on first use and shared thereafter so each
/// test isn't paying for runtime startup.
pub(crate) fn test_runtime() -> &'static tokio::runtime::Runtime {
    static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    TEST_RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort()))
}

/// Build a `HeaderMap` from `(name, value)` pairs. Panics on invalid
/// header names or values — tests should fail loudly on a typo.
pub(crate) fn header_map(entries: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let name: reqwest::header::HeaderName =
            (*name).parse().unwrap_or_else(|_| std::process::abort());
        let value = HeaderValue::from_str(value).unwrap_or_else(|_| std::process::abort());
        headers.insert(name, value);
    }
    headers
}
