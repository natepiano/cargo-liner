//! Direct HTTP client for GitHub and crates.io APIs.
//!
//! Replaces `gh` and `curl` subprocess calls with `ureq` for lower
//! latency (no process-spawn overhead) and connection reuse.

use std::collections::HashMap;
use std::fmt::Write;
use std::process::Command;

use serde::Deserialize;

use super::ci::GhRun;
use super::ci::GqlCheckRun;
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

#[derive(Deserialize)]
struct GqlBatchResponse {
    data: HashMap<String, GqlRunNode>,
}

// ── Client ───────────────────────────────────────────────────────────

/// Shared HTTP client backed by `ureq::Agent` for connection pooling.
/// Clone is cheap — the underlying agent uses `Arc`.
#[derive(Clone)]
pub struct HttpClient {
    agent:        ureq::Agent,
    github_token: Option<String>,
}

impl HttpClient {
    /// Build a new client. Obtains the GitHub auth token from `gh auth
    /// token` (single subprocess call). If `gh` is unavailable or not
    /// authenticated, GitHub API methods degrade gracefully.
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(GH_TIMEOUT))
            .build()
            .new_agent();
        let github_token = Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        Self {
            agent,
            github_token,
        }
    }

    // ── GitHub REST ──────────────────────────────────────────────────

    fn github_get(&self, path: &str) -> Option<Vec<u8>> {
        let token = self.github_token.as_ref()?;
        let url = format!("{GITHUB_API_BASE}/{path}");
        let response = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .call()
            .ok()?;
        let body = response.into_body().read_to_vec().ok()?;
        Some(body)
    }

    // ── GitHub GraphQL ───────────────────────────────────────────────

    fn github_graphql(&self, query: &str) -> Option<Vec<u8>> {
        let token = self.github_token.as_ref()?;
        let payload = serde_json::json!({ "query": query });
        let response = self
            .agent
            .post(GITHUB_GRAPHQL_URL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .send(payload.to_string().as_bytes())
            .ok()?;
        let body = response.into_body().read_to_vec().ok()?;
        Some(body)
    }

    // ── Public API ───────────────────────────────────────────────────

    /// List recent completed workflow runs for a repo.
    pub fn list_runs(
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
        let body = self.github_get(&path)?;
        let response: GhRunsResponse = serde_json::from_slice(&body).ok()?;
        Some(response.workflow_runs)
    }

    /// Batch-fetch job details for multiple runs in a single GraphQL
    /// call. Returns a map from `run_id` → check runs.
    pub fn batch_fetch_jobs(&self, runs: &[&GhRun]) -> HashMap<u64, Vec<GqlCheckRun>> {
        if runs.is_empty() {
            return HashMap::new();
        }

        let fragment = "checkSuite { checkRuns(first: 50) { nodes { \
                        name conclusion startedAt completedAt } } }";

        let aliases: Vec<String> = runs
            .iter()
            .enumerate()
            .map(|(i, run)| {
                format!(
                    "run_{i}: node(id: \"{}\") \
                     {{ ... on WorkflowRun {{ databaseId {fragment} }} }}",
                    run.node_id
                )
            })
            .collect();

        let query = format!("{{ {} }}", aliases.join(" "));
        let Some(body) = self.github_graphql(&query) else {
            return HashMap::new();
        };
        let Ok(response) = serde_json::from_slice::<GqlBatchResponse>(&body) else {
            return HashMap::new();
        };

        response
            .data
            .into_values()
            .filter_map(|node| {
                let check_runs = node.check_suite?.check_runs.nodes;

                Some((node.database_id, check_runs))
            })
            .collect()
    }

    /// Fetch stars and description for a GitHub repo.
    pub fn fetch_repo_meta(&self, owner: &str, repo: &str) -> Option<RepoMetaInfo> {
        let body = self.github_get(&format!("repos/{owner}/{repo}"))?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let stars = json.get("stargazers_count")?.as_u64()?;
        let description = json
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from);
        Some(RepoMetaInfo { stars, description })
    }

    // ── Crates.io ────────────────────────────────────────────────────

    /// Lightweight connectivity probe (HEAD request to crates.io).
    pub fn check_online(&self) -> bool { self.agent.head(CONNECTIVITY_CHECK_URL).call().is_ok() }

    /// Fetch version and download count from the crates.io API.
    pub fn fetch_crates_io_info(&self, crate_name: &str) -> Option<CratesIoInfo> {
        let url = format!("{CRATES_IO_API_BASE}/crates/{crate_name}");
        let response = self
            .agent
            .get(&url)
            .header("User-Agent", CRATES_IO_USER_AGENT)
            .call()
            .ok()?;
        let body = response.into_body().read_to_vec().ok()?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let krate = json.get("crate")?;
        let version = krate
            .get("max_stable_version")?
            .as_str()
            .map(String::from)?;
        let downloads = krate.get("downloads")?.as_u64().unwrap_or(0);
        Some(CratesIoInfo { version, downloads })
    }
}
