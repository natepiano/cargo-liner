//! Direct HTTP client for GitHub and crates.io APIs.
//!
//! Uses `reqwest` (async) backed by a `tokio` runtime for concurrent
//! HTTP. Sync wrappers (`handle.block_on`) are provided for callers
//! that run on std/rayon threads during TUI startup and background work.

use std::collections::HashMap;
use std::fmt::Write;
use std::process::Command;

use serde::Deserialize;

use super::ci::GhRun;
use super::ci::GqlCheckRun;
use super::constants::APP_NAME;
use super::constants::CONNECTIVITY_CHECK_URL;
use super::constants::CRATES_IO_API_BASE;
use super::constants::CRATES_IO_USER_AGENT;
use super::constants::GH_TIMEOUT;
use super::constants::GITHUB_API_BASE;
use super::constants::GITHUB_GRAPHQL_URL;
use super::scan::CratesIoInfo;
use super::scan::RepoMetaInfo;

// ── Serde types for API responses ────────────────────────────────────

#[derive(Deserialize)]
struct GhRunsResponse {
    workflow_runs: Vec<GhRun>,
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
/// pooling and async I/O. Clone is cheap — the underlying client uses
/// `Arc`. A `tokio::runtime::Handle` is stored so sync callers can
/// dispatch async work via `block_on`.
#[derive(Clone)]
pub struct HttpClient {
    client:            reqwest::Client,
    github_token:      Option<String>,
    pub(crate) handle: tokio::runtime::Handle,
}

impl HttpClient {
    /// Build a new client. Obtains the GitHub auth token from `gh auth
    /// token` (single subprocess call). If `gh` is unavailable or not
    /// authenticated, GitHub API methods degrade gracefully.
    pub fn new(handle: tokio::runtime::Handle) -> Option<Self> {
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
            handle,
        })
    }

    // ── Async internals ─────────────────────────────────────────────

    async fn github_get_async(&self, path: &str) -> Option<Vec<u8>> {
        let token = self.github_token.as_ref()?;
        let url = format!("{GITHUB_API_BASE}/{path}");
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .ok()?;
        let body = response.bytes().await.ok()?;
        Some(body.to_vec())
    }

    async fn github_graphql_async(&self, query: &str) -> Option<Vec<u8>> {
        let token = self.github_token.as_ref()?;
        let payload = serde_json::json!({ "query": query });
        let response = self
            .client
            .post(GITHUB_GRAPHQL_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .ok()?;
        let body = response.bytes().await.ok()?;
        Some(body.to_vec())
    }

    // ── Async public API ────────────────────────────────────────────

    /// List recent completed workflow runs for a repo (async).
    pub async fn list_runs_async(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
        count: u32,
    ) -> Option<Vec<GhRun>> {
        let mut path =
            format!("repos/{owner}/{repo}/actions/runs?per_page={count}&status=completed");
        if let Some(branch) = branch {
            let _ = write!(path, "&branch={branch}");
        }
        let body = self.github_get_async(&path).await?;
        let response: GhRunsResponse = serde_json::from_slice(&body).ok()?;
        Some(response.workflow_runs)
    }

    /// Batch-fetch job details for uncached runs AND repo metadata in a
    /// single GraphQL call (async). Returns jobs map + optional repo
    /// metadata.
    pub async fn batch_fetch_jobs_and_meta_async(
        &self,
        owner: &str,
        repo: &str,
        runs: &[&GhRun],
    ) -> (HashMap<u64, Vec<GqlCheckRun>>, Option<RepoMetaInfo>) {
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
        let Some(body) = self.github_graphql_async(&query).await else {
            return (HashMap::new(), None);
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (HashMap::new(), None);
        };
        let Some(data) = json.get("data") else {
            return (HashMap::new(), None);
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

        (jobs, meta)
    }

    /// Lightweight connectivity probe — HEAD request to crates.io
    /// (async).
    pub async fn check_online_async(&self) -> bool {
        self.client
            .head(CONNECTIVITY_CHECK_URL)
            .send()
            .await
            .is_ok()
    }

    /// Fetch version and download count from the crates.io API (async).
    pub async fn fetch_crates_io_info_async(&self, crate_name: &str) -> Option<CratesIoInfo> {
        let url = format!("{CRATES_IO_API_BASE}/crates/{crate_name}");
        let response = self
            .client
            .get(&url)
            .header("User-Agent", CRATES_IO_USER_AGENT)
            .send()
            .await
            .ok()?;
        let body = response.bytes().await.ok()?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let krate = json.get("crate")?;
        let version = krate
            .get("max_stable_version")?
            .as_str()
            .map(String::from)?;
        let downloads = krate.get("downloads")?.as_u64().unwrap_or(0);
        Some(CratesIoInfo { version, downloads })
    }

    // ── Sync wrappers (for std/rayon thread callers) ────────────────

    /// List recent completed workflow runs (sync wrapper).
    pub fn list_runs(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
        count: u32,
    ) -> Option<Vec<GhRun>> {
        self.handle
            .block_on(self.list_runs_async(owner, repo, branch, count))
    }

    /// Batch-fetch job details + repo metadata (sync wrapper).
    pub fn batch_fetch_jobs_and_meta(
        &self,
        owner: &str,
        repo: &str,
        runs: &[&GhRun],
    ) -> (HashMap<u64, Vec<GqlCheckRun>>, Option<RepoMetaInfo>) {
        self.handle
            .block_on(self.batch_fetch_jobs_and_meta_async(owner, repo, runs))
    }

    /// Lightweight connectivity probe (sync wrapper).
    pub fn check_online(&self) -> bool { self.handle.block_on(self.check_online_async()) }

    /// Fetch crates.io info (sync wrapper).
    pub fn fetch_crates_io_info(&self, crate_name: &str) -> Option<CratesIoInfo> {
        self.handle
            .block_on(self.fetch_crates_io_info_async(crate_name))
    }
}

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(GH_TIMEOUT)
        .user_agent(APP_NAME)
        .build()
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    #[allow(clippy::expect_used, reason = "tests should panic on unexpected values")]
    fn client_sends_app_user_agent_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("read listener address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).expect("read request bytes");
            let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
            let response =
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
            stream.write_all(response).expect("write response");
            request
        });

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
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
