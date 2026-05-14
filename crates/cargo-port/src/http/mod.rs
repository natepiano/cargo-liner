//! Direct HTTP client for GitHub and crates.io APIs.
//!
//! Uses `reqwest` (async) backed by a `tokio` runtime for concurrent
//! HTTP. Sync wrappers (`handle.block_on`) are provided for callers
//! that run on std/rayon threads during TUI startup and background work.

mod rate_limit;

use std::collections::HashMap;
use std::fmt::Write;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(crate) use rate_limit::GitHubRateLimit;
pub(crate) use rate_limit::RateLimitBucket;
pub(crate) use rate_limit::RateLimitQuota;
use rate_limit::SYNTHETIC_RATE_LIMIT_SECS;
use rate_limit::classify_network_error;
pub(crate) use rate_limit::github_is_rate_limited;
pub(crate) use rate_limit::graphql_body_is_rate_limited;
use rate_limit::now_epoch_secs;
pub(crate) use rate_limit::parse_rate_limit_headers;
pub(crate) use rate_limit::parse_rate_limit_response;
use reqwest::Client;
use reqwest::Error;
use serde::Deserialize;
use tokio::runtime::Handle;

use super::ci::GhRun;
use super::ci::GqlCheckRun;
use super::constants::APP_NAME;
use super::constants::CRATES_IO_API_BASE;
use super::constants::CRATES_IO_USER_AGENT;
use super::constants::GH_TIMEOUT;
use super::constants::GITHUB_API_BASE;
use super::constants::GITHUB_GRAPHQL_URL;
use super::constants::SERVICE_RETRY_SECS;
use super::scan::CratesIoInfo;
use super::scan::RepoMetaInfo;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum ServiceKind {
    GitHub,
    CratesIo,
}

impl ServiceKind {
    const fn probe_url(self) -> &'static str {
        match self {
            Self::GitHub => GITHUB_API_BASE,
            Self::CratesIo => CRATES_IO_API_BASE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceSignal {
    Reachable(ServiceKind),
    /// The service is unreachable over the network (DNS failure,
    /// connection refused, timeout, 5xx). Distinct from `RateLimited`
    /// because the recovery path and user-facing message differ.
    Unreachable(ServiceKind),
    /// The service is reachable but refusing our requests with a
    /// rate-limit status (GitHub 429, 403 + `X-RateLimit-Remaining: 0`,
    /// or GraphQL body `errors[].type == "RATE_LIMITED"`). The display
    /// buckets can still refresh via the quota-exempt `/rate_limit`
    /// endpoint.
    RateLimited(ServiceKind),
}

type GitHubJobsAndMeta = (HashMap<u64, Vec<GqlCheckRun>>, Option<RepoMetaInfo>);

pub(crate) type HttpOutcome<T> = (Option<T>, Option<ServiceSignal>);

// ── Serde types for API responses ────────────────────────────────────

#[derive(Deserialize)]
struct GhRunsResponse {
    total_count:   u32,
    workflow_runs: Vec<GhRun>,
}

/// Workflow runs plus the total count reported by GitHub.
pub(crate) struct GhRunsList {
    pub runs:        Vec<GhRun>,
    pub total_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRunNode {
    database_id: u64,
    check_suite: Option<GqlCheckSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlCheckSuite {
    check_runs: GqlCheckRunConnection,
}

#[derive(Deserialize)]
struct GqlCheckRunConnection {
    nodes: Vec<GqlCheckRun>,
}

// ── Client ───────────────────────────────────────────────────────────

/// Shared HTTP client backed by `reqwest::Client` for connection
/// pooling and async I/O. `Clone` is cheap — the underlying client uses
/// `Arc`. A `tokio::runtime::Handle` is stored so sync callers can
/// dispatch async work via `block_on`.
#[derive(Clone)]
pub(crate) struct HttpClient {
    client:                  Client,
    github_token:            Option<String>,
    rate_limit:              Arc<Mutex<GitHubRateLimit>>,
    /// When true, every GitHub REST + GraphQL call (and the recovery
    /// probe) short-circuits to a synthetic rate-limited outcome so the
    /// rate-limit UI and toast flow can be exercised deterministically.
    /// `/rate_limit` itself stays real — the display must keep ticking.
    force_github_rate_limit: Arc<AtomicBool>,
    /// Epoch-seconds reset timestamp used to drive the synthetic
    /// core-bucket countdown while `force_github_rate_limit` is on. `0`
    /// means "not set". Rebased on every off→on transition so the
    /// countdown starts at `00:59:59` and ticks down from there.
    force_reset_at:          Arc<AtomicU64>,
    pub(crate) handle:       Handle,
}

impl HttpClient {
    /// Build a new client. Obtains the GitHub auth token from `gh auth
    /// token` (single subprocess call). If `gh` is unavailable or not
    /// authenticated, GitHub API methods degrade gracefully.
    pub(crate) fn new(handle: Handle) -> Option<Self> {
        let client = build_client().ok()?;
        let github_token = Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        Some(Self {
            client,
            github_token,
            rate_limit: Arc::new(Mutex::new(GitHubRateLimit::default())),
            force_github_rate_limit: Arc::new(AtomicBool::new(false)),
            force_reset_at: Arc::new(AtomicU64::new(0)),
            handle,
        })
    }

    /// Toggle the synthetic GitHub rate-limit short-circuit at runtime.
    /// Intended for the `[debug] force_github_rate_limit` config flag.
    /// Turning the flag on rebases the synthetic countdown to
    /// `now + SYNTHETIC_RATE_LIMIT_SECS` so the display starts at
    /// `00:59:59` and counts down from there.
    pub(crate) fn set_force_github_rate_limit(&self, on: bool) {
        self.force_github_rate_limit.store(on, Ordering::Relaxed);
        if on {
            let reset_at = now_epoch_secs().saturating_add(SYNTHETIC_RATE_LIMIT_SECS);
            self.force_reset_at.store(reset_at, Ordering::Relaxed);
        } else {
            self.force_reset_at.store(0, Ordering::Relaxed);
        }
    }

    fn github_rate_limit_forced(&self) -> bool {
        self.force_github_rate_limit.load(Ordering::Relaxed)
    }

    fn synthetic_core_quota(&self) -> RateLimitQuota {
        let reset_at = self.force_reset_at.load(Ordering::Relaxed);
        RateLimitQuota {
            limit:     5000,
            used:      5000,
            remaining: 0,
            reset_at:  if reset_at == 0 { None } else { Some(reset_at) },
        }
    }

    /// the current live rate-limit state. Returned by value —
    /// `GitHubRateLimit` is `Copy`. While `force_github_rate_limit` is
    /// on, the `core` bucket is overridden with a synthetic `0/5000`
    /// reading whose reset timestamp is stable so the countdown ticks
    /// down instead of oscillating. `graphql` stays real so the
    /// live-refresh behaviour of the `/rate_limit` endpoint is still
    /// visible during debug.
    pub(crate) fn rate_limit(&self) -> GitHubRateLimit {
        let real = self
            .rate_limit
            .lock()
            .map(|state| *state)
            .unwrap_or_default();
        if self.github_rate_limit_forced() {
            return GitHubRateLimit {
                core:    Some(self.synthetic_core_quota()),
                graphql: real.graphql,
            };
        }
        real
    }

    fn update_rate_limit_bucket(&self, bucket: RateLimitBucket, quota: RateLimitQuota) {
        let Ok(mut state) = self.rate_limit.lock() else {
            return;
        };
        match bucket {
            RateLimitBucket::Core => state.core = Some(quota),
            RateLimitBucket::GraphQl => state.graphql = Some(quota),
        }
    }

    fn set_rate_limit(&self, github_rate_limit: GitHubRateLimit) {
        if let Ok(mut state) = self.rate_limit.lock() {
            *state = github_rate_limit;
        }
    }

    // ── Async internals ─────────────────────────────────────────────

    async fn github_get_async(&self, path: &str) -> HttpOutcome<Vec<u8>> {
        if self.github_rate_limit_forced() {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        let Some(token) = self.github_token.as_ref() else {
            return (None, None);
        };
        let url = format!("{GITHUB_API_BASE}/{path}");
        let response = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return (None, classify_network_error(ServiceKind::GitHub, &error)),
        };
        if let Some((bucket, quota)) = parse_rate_limit_headers(response.headers()) {
            self.update_rate_limit_bucket(bucket, quota);
        }
        let status = response.status();
        let rate_limited = github_is_rate_limited(status, response.headers());
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    classify_network_error(ServiceKind::GitHub, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::GitHub))),
                );
            },
        };
        if rate_limited {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        (
            Some(body.to_vec()),
            Some(ServiceSignal::Reachable(ServiceKind::GitHub)),
        )
    }

    async fn github_graphql_async(&self, query: &str) -> HttpOutcome<Vec<u8>> {
        if self.github_rate_limit_forced() {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        let Some(token) = self.github_token.as_ref() else {
            return (None, None);
        };
        let payload = serde_json::json!({ "query": query });
        let response = match self
            .client
            .post(GITHUB_GRAPHQL_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return (None, classify_network_error(ServiceKind::GitHub, &error)),
        };
        if let Some((bucket, quota)) = parse_rate_limit_headers(response.headers()) {
            self.update_rate_limit_bucket(bucket, quota);
        }
        let status = response.status();
        let http_rate_limited = github_is_rate_limited(status, response.headers());
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    classify_network_error(ServiceKind::GitHub, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::GitHub))),
                );
            },
        };
        if http_rate_limited {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        // GraphQL returns HTTP 200 on rate-limit, so status-code
        // detection alone is insufficient — inspect the body's
        // `errors[].type` for `RATE_LIMITED`.
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body)
            && graphql_body_is_rate_limited(&json)
        {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        (
            Some(body.to_vec()),
            Some(ServiceSignal::Reachable(ServiceKind::GitHub)),
        )
    }

    // ── Async public API ────────────────────────────────────────────

    /// List recent completed workflow runs for a repo (async).
    pub(crate) async fn list_runs_async(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
        count: u32,
        created_before: Option<&str>,
    ) -> HttpOutcome<GhRunsList> {
        let mut path =
            format!("repos/{owner}/{repo}/actions/runs?per_page={count}&status=completed");
        if let Some(branch) = branch {
            let _ = write!(path, "&branch={branch}");
        }
        // ISO 8601 timestamp from CiRun.created_at — strict less-than.
        if let Some(date) = created_before {
            let _ = write!(path, "&created=<{date}");
        }
        let (body, signal) = self.github_get_async(&path).await;
        let value = body.and_then(|body| {
            serde_json::from_slice::<GhRunsResponse>(&body)
                .ok()
                .map(|response| GhRunsList {
                    runs:        response.workflow_runs,
                    total_count: response.total_count,
                })
        });
        (value, signal)
    }

    /// Batch-fetch job details for uncached runs AND repo metadata in a
    /// single GraphQL call (async). Returns jobs map + optional repo
    /// metadata.
    pub(crate) async fn batch_fetch_jobs_and_meta_async(
        &self,
        owner: &str,
        repo: &str,
        runs: &[&GhRun],
    ) -> HttpOutcome<GitHubJobsAndMeta> {
        let repo_fragment = format!(
            "repo: repository(owner: \"{owner}\", name: \"{repo}\") {{ stargazerCount description }}"
        );

        let run_fragment = "checkSuite { checkRuns(first: 50) { nodes { \
                            name conclusion startedAt completedAt } } }";

        let mut parts = vec![repo_fragment];
        for (i, run) in runs.iter().enumerate() {
            parts.push(format!(
                "run_{i}: node(id: \"{}\") \
                 {{ ... on WorkflowRun {{ databaseId {run_fragment} }} }}",
                run.node_id
            ));
        }

        let query = format!("{{ {} }}", parts.join(" "));
        let (body, signal) = self.github_graphql_async(&query).await;
        let Some(body) = body else {
            return (None, signal);
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (None, signal);
        };
        let Some(data) = json.get("data") else {
            return (None, signal);
        };

        // Parse repo metadata.
        let meta = data.get("repo").and_then(|r| {
            let stars = r.get("stargazerCount")?.as_u64()?;
            let description = r
                .get("description")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from);
            Some(RepoMetaInfo { stars, description })
        });

        // Parse run nodes.
        let jobs = data
            .as_object()
            .map(|obj| {
                obj.iter()
                    .filter(|(key, _)| key.starts_with("run_"))
                    .filter_map(|(_, val)| {
                        let node: GqlRunNode = serde_json::from_value(val.clone()).ok()?;
                        let check_runs = node.check_suite?.check_runs.nodes;
                        Some((node.database_id, check_runs))
                    })
                    .collect()
            })
            .unwrap_or_default();

        (Some((jobs, meta)), signal)
    }

    /// Call GitHub's `/rate_limit` endpoint, which is itself exempt from
    /// the quota and therefore safe to poll while we're rate-limited.
    /// Updates the shared live `rate_limit` on success.
    pub(crate) async fn fetch_rate_limit_async(&self) -> HttpOutcome<GitHubRateLimit> {
        let Some(token) = self.github_token.as_ref() else {
            return (None, None);
        };
        let url = format!("{GITHUB_API_BASE}/rate_limit");
        let response = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return (None, classify_network_error(ServiceKind::GitHub, &error)),
        };
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    classify_network_error(ServiceKind::GitHub, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::GitHub))),
                );
            },
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::GitHub)));
        };
        let github_rate_limit = parse_rate_limit_response(&json);
        self.set_rate_limit(github_rate_limit);
        (
            Some(github_rate_limit),
            Some(ServiceSignal::Reachable(ServiceKind::GitHub)),
        )
    }

    /// Recovery probe used while retrying after an `Unreachable` signal.
    ///
    /// For GitHub, a plain `HEAD https://api.github.com` is not enough —
    /// it returns 200 even when fully rate-limited (no auth, no quota
    /// debit). That made "recovery" fire within ~100ms of every
    /// Unreachable signal, dismissing the toast only to have it
    /// immediately re-created by the next 429. Use `/rate_limit` (which
    /// is exempt from the quota) and treat the service as recovered
    /// only when both core and graphql have at least 1 request
    /// remaining. While the debug force flag is on, GitHub never
    /// recovers — the probe always returns `false`.
    pub(crate) async fn probe_service_async(&self, service: ServiceKind) -> bool {
        match service {
            ServiceKind::GitHub => self.probe_github_rate_limit_async().await,
            ServiceKind::CratesIo => self
                .client
                .head(service.probe_url())
                .timeout(Duration::from_secs(SERVICE_RETRY_SECS))
                .send()
                .await
                .is_ok(),
        }
    }

    async fn probe_github_rate_limit_async(&self) -> bool {
        let (github_rate_limit, _signal) = self.fetch_rate_limit_async().await;
        if self.github_rate_limit_forced() {
            // Forced mode: display keeps updating via /rate_limit above,
            // but never report recovery — the error toast must persist
            // for testing.
            return false;
        }
        match github_rate_limit {
            Some(s) => {
                s.core.is_some_and(|q| q.remaining > 0)
                    && s.graphql.is_some_and(|q| q.remaining > 0)
            },
            None => self
                .client
                .head(ServiceKind::GitHub.probe_url())
                .timeout(Duration::from_secs(SERVICE_RETRY_SECS))
                .send()
                .await
                .is_ok(),
        }
    }

    /// Fetch version and download count from the crates.io API (async).
    pub(crate) async fn fetch_crates_io_info_async(
        &self,
        crate_name: &str,
    ) -> HttpOutcome<CratesIoInfo> {
        let url = format!("{CRATES_IO_API_BASE}/crates/{crate_name}");
        let response = match self
            .client
            .get(&url)
            .header("User-Agent", CRATES_IO_USER_AGENT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return (None, classify_network_error(ServiceKind::CratesIo, &error)),
        };
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    classify_network_error(ServiceKind::CratesIo, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::CratesIo))),
                );
            },
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        let Some(krate) = json.get("crate") else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        let Some(max_stable_version) = krate.get("max_stable_version") else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        let Some(version) = max_stable_version.as_str().map(String::from) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        let downloads = krate
            .get("downloads")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        (
            Some(CratesIoInfo { version, downloads }),
            Some(ServiceSignal::Reachable(ServiceKind::CratesIo)),
        )
    }

    // ── Sync wrappers (for std/rayon thread callers) ────────────────

    /// List recent completed workflow runs (sync wrapper).
    pub(crate) fn list_runs(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
        count: u32,
        created_before: Option<&str>,
    ) -> HttpOutcome<GhRunsList> {
        self.handle
            .block_on(self.list_runs_async(owner, repo, branch, count, created_before))
    }

    /// Batch-fetch job details + repo metadata (sync wrapper).
    pub(crate) fn batch_fetch_jobs_and_meta(
        &self,
        owner: &str,
        repo: &str,
        runs: &[&GhRun],
    ) -> HttpOutcome<GitHubJobsAndMeta> {
        self.handle
            .block_on(self.batch_fetch_jobs_and_meta_async(owner, repo, runs))
    }

    pub(crate) fn probe_service(&self, service: ServiceKind) -> bool {
        self.handle.block_on(self.probe_service_async(service))
    }

    /// Fetch `/rate_limit` (sync wrapper).
    pub(crate) fn fetch_rate_limit(&self) -> HttpOutcome<GitHubRateLimit> {
        self.handle.block_on(self.fetch_rate_limit_async())
    }

    /// Fetch crates.io info (sync wrapper).
    pub(crate) fn fetch_crates_io_info(&self, crate_name: &str) -> HttpOutcome<CratesIoInfo> {
        self.handle
            .block_on(self.fetch_crates_io_info_async(crate_name))
    }
}

fn build_client() -> Result<Client, Error> {
    reqwest::Client::builder()
        .timeout(GH_TIMEOUT)
        .user_agent(APP_NAME)
        .build()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Read;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;

    use reqwest::StatusCode;
    use reqwest::header::HeaderMap;
    use serde_json::json;
    use tokio::runtime::Handle;

    use super::*;
    use crate::test_support;

    #[test]
    fn rate_limit_headers_core_bucket_parsed() {
        let headers = test_support::header_map(&[
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-used", "42"),
            ("x-ratelimit-remaining", "4958"),
            ("x-ratelimit-reset", "1717000000"),
        ]);
        let (bucket, quota) = parse_rate_limit_headers(&headers).unwrap();
        assert_eq!(bucket, RateLimitBucket::Core);
        assert_eq!(quota.limit, 5000);
        assert_eq!(quota.used, 42);
        assert_eq!(quota.remaining, 4958);
        assert_eq!(quota.reset_at, Some(1_717_000_000));
    }

    #[test]
    fn rate_limit_headers_graphql_bucket_parsed() {
        let headers = test_support::header_map(&[
            ("x-ratelimit-resource", "graphql"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-used", "12"),
            ("x-ratelimit-remaining", "4988"),
            ("x-ratelimit-reset", "1717000000"),
        ]);
        let (bucket, _) = parse_rate_limit_headers(&headers).unwrap();
        assert_eq!(bucket, RateLimitBucket::GraphQl);
    }

    #[test]
    fn rate_limit_headers_missing_are_none() {
        let headers = test_support::header_map(&[("x-ratelimit-resource", "core")]);
        assert!(parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn rate_limit_headers_unknown_bucket_is_none() {
        let headers = test_support::header_map(&[
            ("x-ratelimit-resource", "search"),
            ("x-ratelimit-limit", "30"),
            ("x-ratelimit-used", "0"),
            ("x-ratelimit-remaining", "30"),
        ]);
        assert!(parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn parse_rate_limit_response_parses_both_buckets() {
        let body = json!({
            "resources": {
                "core":    { "limit": 5000, "used": 42,  "remaining": 4958, "reset": 1_717_000_000 },
                "graphql": { "limit": 5000, "used": 12,  "remaining": 4988, "reset": 1_717_000_000 },
            },
        });
        let github_rate_limit = parse_rate_limit_response(&body);
        let core = github_rate_limit.core.unwrap();
        assert_eq!(core.limit, 5000);
        assert_eq!(core.used, 42);
        assert_eq!(core.remaining, 4958);
        assert_eq!(core.reset_at, Some(1_717_000_000));
        let gql = github_rate_limit.graphql.unwrap();
        assert_eq!(gql.limit, 5000);
        assert_eq!(gql.remaining, 4988);
    }

    #[test]
    fn parse_rate_limit_response_missing_bucket_is_none() {
        let body = json!({
            "resources": {
                "core": { "limit": 5000, "used": 0, "remaining": 5000 },
            },
        });
        let github_rate_limit = parse_rate_limit_response(&body);
        assert!(github_rate_limit.core.is_some());
        assert!(github_rate_limit.graphql.is_none());
    }

    #[test]
    fn github_is_rate_limited_on_429() {
        let headers = HeaderMap::new();
        assert!(github_is_rate_limited(
            StatusCode::TOO_MANY_REQUESTS,
            &headers
        ));
    }

    #[test]
    fn github_is_rate_limited_on_403_with_zero_remaining() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "0")]);
        assert!(github_is_rate_limited(StatusCode::FORBIDDEN, &headers));
    }

    #[test]
    fn github_is_not_rate_limited_on_403_with_remaining() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "500")]);
        assert!(!github_is_rate_limited(StatusCode::FORBIDDEN, &headers));
    }

    #[test]
    fn github_is_not_rate_limited_on_200() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "0")]);
        assert!(!github_is_rate_limited(StatusCode::OK, &headers));
    }

    #[test]
    fn graphql_rate_limited_body_is_detected() {
        let body = json!({ "errors": [{ "type": "RATE_LIMITED", "message": "x" }] });
        assert!(graphql_body_is_rate_limited(&body));
    }

    #[test]
    fn graphql_body_without_errors_is_not_rate_limited() {
        let body = json!({ "data": { "repo": null } });
        assert!(!graphql_body_is_rate_limited(&body));
    }

    #[test]
    fn graphql_body_with_unrelated_errors_is_not_rate_limited() {
        let body = json!({ "errors": [{ "type": "NOT_FOUND", "message": "x" }] });
        assert!(!graphql_body_is_rate_limited(&body));
    }

    fn test_client(handle: &Handle) -> HttpClient {
        HttpClient {
            client:                  build_client().expect("build http client"),
            github_token:            None,
            rate_limit:              Arc::new(Mutex::new(GitHubRateLimit::default())),
            force_github_rate_limit: Arc::new(AtomicBool::new(false)),
            force_reset_at:          Arc::new(AtomicU64::new(0)),
            handle:                  handle.clone(),
        }
    }

    #[test]
    fn force_rate_limit_synthesizes_zero_core_with_future_reset() {
        let runtime = test_support::test_runtime();
        let client = test_client(runtime.handle());
        let real_graphql = RateLimitQuota {
            limit:     5000,
            used:      12,
            remaining: 4988,
            reset_at:  Some(1_800_000_000),
        };
        client.update_rate_limit_bucket(RateLimitBucket::GraphQl, real_graphql);

        let before = now_epoch_secs();
        client.set_force_github_rate_limit(true);
        let after = now_epoch_secs();

        let github_rate_limit = client.rate_limit();
        let core = github_rate_limit.core.expect("synthetic core bucket");
        assert_eq!(core.limit, 5000);
        assert_eq!(core.remaining, 0);
        assert_eq!(core.used, 5000);
        let reset_at = core.reset_at.expect("synthetic reset_at");
        assert!(reset_at >= before + SYNTHETIC_RATE_LIMIT_SECS);
        assert!(reset_at <= after + SYNTHETIC_RATE_LIMIT_SECS);

        // `graphql` stays real so the live-refresh path is still
        // observable during debug.
        assert_eq!(github_rate_limit.graphql, Some(real_graphql));

        client.set_force_github_rate_limit(false);
        let github_rate_limit = client.rate_limit();
        // Real core was never populated, so clearing force leaves it
        // at `None`.
        assert!(github_rate_limit.core.is_none());
        assert_eq!(github_rate_limit.graphql, Some(real_graphql));
    }

    #[test]
    fn rate_limit_reflects_bucket_updates() {
        let runtime = test_support::test_runtime();
        let client = test_client(runtime.handle());

        assert_eq!(client.rate_limit(), GitHubRateLimit::default());

        let core_quota = RateLimitQuota {
            limit:     5000,
            used:      42,
            remaining: 4958,
            reset_at:  Some(1_717_000_000),
        };
        client.update_rate_limit_bucket(RateLimitBucket::Core, core_quota);
        let github_rate_limit = client.rate_limit();
        assert_eq!(github_rate_limit.core, Some(core_quota));
        assert!(github_rate_limit.graphql.is_none());

        let gql_quota = RateLimitQuota {
            limit:     5000,
            used:      1,
            remaining: 4999,
            reset_at:  None,
        };
        client.update_rate_limit_bucket(RateLimitBucket::GraphQl, gql_quota);
        let github_rate_limit = client.rate_limit();
        assert_eq!(github_rate_limit.core, Some(core_quota));
        assert_eq!(github_rate_limit.graphql, Some(gql_quota));
    }

    #[test]
    fn client_sends_app_user_agent_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("read listener address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).expect("read request bytes");
            let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
            stream.write_all(response).expect("write response");
            request
        });

        let runtime = test_support::test_runtime();
        let client = build_client().expect("build http client");
        let url = format!("http://{addr}/");
        let response = runtime
            .block_on(async { client.get(url).send().await })
            .expect("send request");
        assert!(response.status().is_success());

        let request = server.join().expect("join server thread");
        assert!(
            request.contains(&format!("user-agent: {APP_NAME}\r\n"))
                || request.contains(&format!("User-Agent: {APP_NAME}\r\n")),
            "expected request to include User-Agent header, got:\n{request}"
        );
    }
}
