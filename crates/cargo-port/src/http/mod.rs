//! Direct HTTP client for GitHub and crates.io APIs.
//!
//! Uses `reqwest` (async) backed by a `tokio` runtime for concurrent
//! HTTP. Sync wrappers (`handle.block_on`) are provided for callers
//! that run on std/rayon threads during TUI startup and background work.

mod auth;
mod client;
mod constants;
mod crates_io;
mod github_actions;
mod github_pull_requests;
mod rate_limit;

pub(crate) use auth::GithubAuthGap;
pub(crate) use client::HttpClient;
pub(crate) use client::ServiceKind;
pub(crate) use client::ServiceSignal;
pub(crate) use github_pull_requests::PullRequestFetch;
pub(crate) use rate_limit::GitHubRateLimit;
pub(crate) use rate_limit::RateLimitQuota;
