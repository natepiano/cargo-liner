use std::collections::HashMap;
use std::fmt::Write;

use serde::Deserialize;

use super::client::HttpClient;
use super::client::HttpOutcome;
use super::constants::GITHUB_GRAPHQL_DATA_KEY;
use super::constants::GITHUB_GRAPHQL_DESCRIPTION_KEY;
use super::constants::GITHUB_GRAPHQL_REPO_KEY;
use super::constants::GITHUB_GRAPHQL_RUN_ALIAS_PREFIX;
use super::constants::GITHUB_GRAPHQL_STARGAZER_COUNT_KEY;
use super::constants::GITHUB_PR_PAGE_SIZE;
use crate::ci::GhRun;
use crate::ci::GqlCheckRun;
use crate::scan::RepoMetaInfo;

pub(crate) type GitHubJobsAndMeta = (HashMap<u64, Vec<GqlCheckRun>>, Option<RepoMetaInfo>);
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

impl HttpClient {
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

        let run_fragment = format!(
            "checkSuite {{ checkRuns(first: {GITHUB_PR_PAGE_SIZE}) {{ nodes {{ \
             name conclusion startedAt completedAt }} }} }}"
        );

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
        let Some(data) = json.get(GITHUB_GRAPHQL_DATA_KEY) else {
            return (None, signal);
        };

        // Parse repo metadata.
        let meta = data.get(GITHUB_GRAPHQL_REPO_KEY).and_then(|r| {
            let stars = r.get(GITHUB_GRAPHQL_STARGAZER_COUNT_KEY)?.as_u64()?;
            let description = r
                .get(GITHUB_GRAPHQL_DESCRIPTION_KEY)
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
                    .filter(|(key, _)| key.starts_with(GITHUB_GRAPHQL_RUN_ALIAS_PREFIX))
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
}
