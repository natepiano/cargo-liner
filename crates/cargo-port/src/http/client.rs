use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use reqwest::Client;
use reqwest::Error;
use tokio::runtime::Handle;

use super::auth::GithubAuth;
use super::auth::GithubAuthGap;
use super::constants::ACCEPT_HEADER;
use super::constants::AUTHORIZATION_HEADER;
use super::constants::CONTENT_TYPE_HEADER;
use super::constants::GITHUB_CORE_RATE_LIMIT_CAP;
use super::constants::GITHUB_JSON_MEDIA_TYPE;
use super::constants::JSON_MEDIA_TYPE;
use super::rate_limit;
use super::rate_limit::GitHubRateLimit;
use super::rate_limit::RateLimitBucket;
use super::rate_limit::RateLimitQuota;
use super::rate_limit::SYNTHETIC_RATE_LIMIT_SECS;
use crate::constants::APP_NAME;
use crate::constants::CRATES_IO_API_BASE;
use crate::constants::GH_TIMEOUT;
use crate::constants::GITHUB_API_BASE;
use crate::constants::GITHUB_GRAPHQL_URL;
use crate::constants::SERVICE_RETRY_SECS;

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
pub(crate) type HttpOutcome<T> = (Option<T>, Option<ServiceSignal>);
// ── Client ───────────────────────────────────────────────────────────

/// Shared HTTP client backed by `reqwest::Client` for connection
/// pooling and async I/O. `Clone` is cheap — the underlying client uses
/// `Arc`. A `tokio::runtime::Handle` is stored so sync callers can
/// dispatch async work via `block_on`.
#[derive(Clone)]
pub(crate) struct HttpClient {
    pub(super) client:                  Client,
    pub(super) github_auth:             GithubAuth,
    pub(super) github_viewer_login:     Arc<Mutex<Option<String>>>,
    pub(super) rate_limit:              Arc<Mutex<GitHubRateLimit>>,
    /// When true, every GitHub REST + GraphQL call (and the recovery
    /// probe) short-circuits to a synthetic rate-limited outcome so the
    /// rate-limit UI and toast flow can be exercised deterministically.
    /// `/rate_limit` itself stays real — the display must keep ticking.
    pub(super) force_github_rate_limit: Arc<AtomicBool>,
    /// Epoch-seconds reset timestamp used to drive the synthetic
    /// core-bucket countdown while `force_github_rate_limit` is on. `0`
    /// means "not set". Rebased on every off→on transition so the
    /// countdown starts at `00:59:59` and ticks down from there.
    pub(super) force_reset_at:          Arc<AtomicU64>,
    pub(crate) handle:                  Handle,
}

impl HttpClient {
    /// Build a new client. Obtains the GitHub auth token from `gh auth
    /// token` (single subprocess call). If `gh` is unavailable or not
    /// authenticated, GitHub API methods degrade gracefully.
    pub(crate) fn new(handle: Handle) -> Option<Self> {
        let client = build_client().ok()?;
        let github_auth = GithubAuth::classify(Command::new("gh").args(["auth", "token"]).output());
        Some(Self {
            client,
            github_auth,
            github_viewer_login: Arc::new(Mutex::new(None)),
            rate_limit: Arc::new(Mutex::new(GitHubRateLimit::default())),
            force_github_rate_limit: Arc::new(AtomicBool::new(false)),
            force_reset_at: Arc::new(AtomicU64::new(0)),
            handle,
        })
    }

    /// Whether a GitHub auth token was obtained at construction. When
    /// false, every authenticated REST / GraphQL call short-circuits to
    /// a no-op (see `github_get_async` / `github_graphql_async`), so CI
    /// runs and rate-limit buckets never load.
    pub(crate) const fn has_github_token(&self) -> bool {
        matches!(self.github_auth, GithubAuth::Authenticated(_))
    }

    /// The GitHub auth gap to surface at startup, or `None` when a token
    /// was obtained. Drives the startup toast copy and the git-pane row.
    pub(crate) const fn github_auth_gap(&self) -> Option<GithubAuthGap> { self.github_auth.gap() }

    /// Toggle the synthetic GitHub rate-limit short-circuit at runtime.
    /// Intended for the `[debug] force_github_rate_limit` config flag.
    /// Turning the flag on rebases the synthetic countdown to
    /// `now + SYNTHETIC_RATE_LIMIT_SECS` so the display starts at
    /// `00:59:59` and counts down from there.
    pub(crate) fn set_force_github_rate_limit(&self, on: bool) {
        self.force_github_rate_limit.store(on, Ordering::Relaxed);
        if on {
            let reset_at = rate_limit::now_epoch_secs().saturating_add(SYNTHETIC_RATE_LIMIT_SECS);
            self.force_reset_at.store(reset_at, Ordering::Relaxed);
        } else {
            self.force_reset_at.store(0, Ordering::Relaxed);
        }
    }

    pub(super) fn github_rate_limit_forced(&self) -> bool {
        self.force_github_rate_limit.load(Ordering::Relaxed)
    }

    fn synthetic_core_quota(&self) -> RateLimitQuota {
        let reset_at = self.force_reset_at.load(Ordering::Relaxed);
        RateLimitQuota {
            limit:     GITHUB_CORE_RATE_LIMIT_CAP,
            used:      GITHUB_CORE_RATE_LIMIT_CAP,
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

    pub(super) fn update_rate_limit_bucket(&self, bucket: RateLimitBucket, quota: RateLimitQuota) {
        let Ok(mut state) = self.rate_limit.lock() else {
            return;
        };
        match bucket {
            RateLimitBucket::Core => state.core = Some(quota),
            RateLimitBucket::GraphQl => state.graphql = Some(quota),
        }
    }

    pub(super) fn set_rate_limit(&self, github_rate_limit: GitHubRateLimit) {
        if let Ok(mut state) = self.rate_limit.lock() {
            *state = github_rate_limit;
        }
    }

    // ── Async internals ─────────────────────────────────────────────

    pub(super) async fn github_get_async(&self, path: &str) -> HttpOutcome<Vec<u8>> {
        if self.github_rate_limit_forced() {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        let Some(token) = self.github_auth.token() else {
            return (None, None);
        };
        let url = format!("{GITHUB_API_BASE}/{path}");
        let response = match self
            .client
            .get(&url)
            .header(AUTHORIZATION_HEADER, format!("Bearer {token}"))
            .header(ACCEPT_HEADER, GITHUB_JSON_MEDIA_TYPE)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error),
                );
            },
        };
        if let Some((bucket, quota)) = rate_limit::parse_rate_limit_headers(response.headers()) {
            self.update_rate_limit_bucket(bucket, quota);
        }
        let status = response.status();
        let rate_limited = rate_limit::github_is_rate_limited(status, response.headers());
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error)
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

    pub(super) async fn github_graphql_async(&self, query: &str) -> HttpOutcome<Vec<u8>> {
        if self.github_rate_limit_forced() {
            return (None, Some(ServiceSignal::RateLimited(ServiceKind::GitHub)));
        }
        let Some(token) = self.github_auth.token() else {
            return (None, None);
        };
        let payload = serde_json::json!({ "query": query });
        let response = match self
            .client
            .post(GITHUB_GRAPHQL_URL)
            .header(AUTHORIZATION_HEADER, format!("Bearer {token}"))
            .header(CONTENT_TYPE_HEADER, JSON_MEDIA_TYPE)
            .body(payload.to_string())
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error),
                );
            },
        };
        if let Some((bucket, quota)) = rate_limit::parse_rate_limit_headers(response.headers()) {
            self.update_rate_limit_bucket(bucket, quota);
        }
        let status = response.status();
        let http_rate_limited = rate_limit::github_is_rate_limited(status, response.headers());
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error)
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
            && rate_limit::graphql_body_is_rate_limited(&json)
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
    pub(crate) async fn fetch_rate_limit_async(&self) -> HttpOutcome<GitHubRateLimit> {
        let Some(token) = self.github_auth.token() else {
            return (None, None);
        };
        let url = format!("{GITHUB_API_BASE}/rate_limit");
        let response = match self
            .client
            .get(&url)
            .header(AUTHORIZATION_HEADER, format!("Bearer {token}"))
            .header(ACCEPT_HEADER, GITHUB_JSON_MEDIA_TYPE)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error),
                );
            },
        };
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::GitHub, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::GitHub))),
                );
            },
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::GitHub)));
        };
        let github_rate_limit = rate_limit::parse_rate_limit_response(&json);
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
}

impl HttpClient {
    pub(crate) fn probe_service(&self, service: ServiceKind) -> bool {
        self.handle.block_on(self.probe_service_async(service))
    }

    /// Fetch `/rate_limit` (sync wrapper).
    pub(crate) fn fetch_rate_limit(&self) -> HttpOutcome<GitHubRateLimit> {
        self.handle.block_on(self.fetch_rate_limit_async())
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
        let (bucket, quota) = rate_limit::parse_rate_limit_headers(&headers).unwrap();
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
        let (bucket, _) = rate_limit::parse_rate_limit_headers(&headers).unwrap();
        assert_eq!(bucket, RateLimitBucket::GraphQl);
    }

    #[test]
    fn rate_limit_headers_missing_are_none() {
        let headers = test_support::header_map(&[("x-ratelimit-resource", "core")]);
        assert!(rate_limit::parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn rate_limit_headers_unknown_bucket_is_none() {
        let headers = test_support::header_map(&[
            ("x-ratelimit-resource", "search"),
            ("x-ratelimit-limit", "30"),
            ("x-ratelimit-used", "0"),
            ("x-ratelimit-remaining", "30"),
        ]);
        assert!(rate_limit::parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn parse_rate_limit_response_parses_both_buckets() {
        let body = json!({
            "resources": {
                "core":    { "limit": 5000, "used": 42,  "remaining": 4958, "reset": 1_717_000_000 },
                "graphql": { "limit": 5000, "used": 12,  "remaining": 4988, "reset": 1_717_000_000 },
            },
        });
        let github_rate_limit = rate_limit::parse_rate_limit_response(&body);
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
        let github_rate_limit = rate_limit::parse_rate_limit_response(&body);
        assert!(github_rate_limit.core.is_some());
        assert!(github_rate_limit.graphql.is_none());
    }

    #[test]
    fn github_is_rate_limited_on_429() {
        let headers = HeaderMap::new();
        assert!(rate_limit::github_is_rate_limited(
            StatusCode::TOO_MANY_REQUESTS,
            &headers
        ));
    }

    #[test]
    fn github_is_rate_limited_on_403_with_zero_remaining() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "0")]);
        assert!(rate_limit::github_is_rate_limited(
            StatusCode::FORBIDDEN,
            &headers
        ));
    }

    #[test]
    fn github_is_not_rate_limited_on_403_with_remaining() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "500")]);
        assert!(!rate_limit::github_is_rate_limited(
            StatusCode::FORBIDDEN,
            &headers
        ));
    }

    #[test]
    fn github_is_not_rate_limited_on_200() {
        let headers = test_support::header_map(&[("x-ratelimit-remaining", "0")]);
        assert!(!rate_limit::github_is_rate_limited(
            StatusCode::OK,
            &headers
        ));
    }

    #[test]
    fn graphql_rate_limited_body_is_detected() {
        let body = json!({ "errors": [{ "type": "RATE_LIMITED", "message": "x" }] });
        assert!(rate_limit::graphql_body_is_rate_limited(&body));
    }

    #[test]
    fn graphql_body_without_errors_is_not_rate_limited() {
        let body = json!({ "data": { "repo": null } });
        assert!(!rate_limit::graphql_body_is_rate_limited(&body));
    }

    #[test]
    fn graphql_body_with_unrelated_errors_is_not_rate_limited() {
        let body = json!({ "errors": [{ "type": "NOT_FOUND", "message": "x" }] });
        assert!(!rate_limit::graphql_body_is_rate_limited(&body));
    }

    fn test_client(handle: &Handle) -> HttpClient {
        HttpClient {
            client:                  build_client().expect("build http client"),
            github_auth:             GithubAuth::Unauthenticated,
            github_viewer_login:     Arc::new(Mutex::new(None)),
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

        let before = rate_limit::now_epoch_secs();
        client.set_force_github_rate_limit(true);
        let after = rate_limit::now_epoch_secs();

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
