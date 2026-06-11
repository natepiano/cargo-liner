// cargo metadata

pub(super) const CARGO_OFFLINE_FLAG: &str = "--offline";

// src scan test_counts
/// `#[test]`-family attribute paths counted as test functions. Each entry
/// is the path as written between `#[` and the following `]` or `(`, so
/// `#[tokio::test]` matches `tokio::test` and `#[test_case(...)]` matches
/// `test_case`. `#[cfg(test)]` is deliberately absent — its attribute
/// path is `cfg`, not `test`, so it never matches.
pub(super) const TEST_ATTRIBUTES: [&str; 6] = [
    "test",
    "tokio::test",
    "async_std::test",
    "rstest",
    "test_case",
    "googletest::test",
];
