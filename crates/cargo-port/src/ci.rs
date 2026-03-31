use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::process::Command;
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use serde::Deserialize;
use serde::Serialize;

use super::constants::CANCELLED;
use super::constants::FAILING;
use super::constants::GH_TIMEOUT;
use super::constants::PASSING;
use super::output;

#[derive(Args)]
pub struct CiArgs {
    /// Filter to a specific branch
    #[arg(long, short)]
    branch: Option<String>,

    /// Number of recent runs to show
    #[arg(long, short = 'n', default_value = "1")]
    count: u32,

    /// Output as JSON instead of a table
    #[arg(long)]
    json: bool,
}

/// Workflow run from the GitHub REST API (`/actions/runs`).
#[derive(Deserialize)]
pub struct GhRun {
    pub id:            u64,
    pub node_id:       String,
    pub created_at:    String,
    pub head_branch:   String,
    pub display_title: Option<String>,
}

/// Response wrapper for the REST API runs listing.
#[derive(Deserialize)]
struct GhRunsResponse {
    workflow_runs: Vec<GhRun>,
}

/// Job from the GraphQL `checkRuns` response (upper-case conclusion).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlCheckRun {
    name:         String,
    conclusion:   Option<String>,
    started_at:   Option<String>,
    completed_at: Option<String>,
}

/// GraphQL node response containing a check suite with jobs.
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

/// Top-level GraphQL response with aliased run nodes.
#[derive(Deserialize)]
struct GqlBatchResponse {
    data: HashMap<String, GqlRunNode>,
}

#[derive(Deserialize)]
struct GhRepo {
    url: String,
}

/// Whether a CI run has been fully fetched from the API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub enum FetchStatus {
    #[default]
    Fetched,
    Pending,
}

impl From<bool> for FetchStatus {
    fn from(b: bool) -> Self { if b { Self::Fetched } else { Self::Pending } }
}

impl From<FetchStatus> for bool {
    fn from(val: FetchStatus) -> Self { matches!(val, FetchStatus::Fetched) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conclusion {
    Success,
    Failure,
    Cancelled,
}

impl Conclusion {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Success => PASSING,
            Self::Failure => FAILING,
            Self::Cancelled => CANCELLED,
        }
    }

    pub const fn is_success(self) -> bool { matches!(self, Self::Success) }

    pub const fn is_failure(self) -> bool { matches!(self, Self::Failure) }
}

impl std::fmt::Display for Conclusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.icon()) }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CiRun {
    pub run_id:          u64,
    pub created_at:      String,
    pub branch:          String,
    pub url:             String,
    pub conclusion:      Conclusion,
    pub jobs:            Vec<CiJob>,
    pub wall_clock_secs: Option<u64>,
    #[serde(default)]
    pub commit_title:    Option<String>,
    #[serde(default)]
    pub fetched:         FetchStatus,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CiJob {
    pub name:          String,
    pub conclusion:    Conclusion,
    pub duration:      String,
    pub duration_secs: Option<u64>,
}

pub fn run(path: &Path, args: &CiArgs) -> ExitCode {
    let Ok(repo_dir) = path.canonicalize() else {
        eprintln!("Error: cannot resolve path '{}'", path.display());
        return ExitCode::FAILURE;
    };

    let Some(repo_url) = get_repo_url(&repo_dir) else {
        eprintln!("Error: failed to get repo URL — is this a GitHub repo with `gh` installed?");
        return ExitCode::FAILURE;
    };

    let runs = match list_runs(&repo_dir, args.branch.as_ref(), args.count) {
        Some(runs) if !runs.is_empty() => runs,
        _ => {
            match &args.branch {
                Some(branch) => eprintln!("No completed runs found on branch: {branch}"),
                None => eprintln!("No completed runs found"),
            }
            return ExitCode::FAILURE;
        },
    };

    let run_refs: Vec<&GhRun> = runs.iter().collect();
    let jobs_map = batch_fetch_jobs(&repo_dir, &run_refs);
    let ci_runs: Vec<CiRun> = runs
        .iter()
        .filter_map(|gh_run| {
            let check_runs = jobs_map.get(&gh_run.id)?;
            Some(build_ci_run(gh_run, check_runs.clone(), &repo_url))
        })
        .collect();

    if args.json {
        output::render_ci_json(&ci_runs);
    } else {
        output::render_ci_table(&ci_runs);
    }

    ExitCode::SUCCESS
}

/// Build a `CiRun` from a `GhRun` and pre-fetched check run data.
pub fn build_ci_run(gh_run: &GhRun, check_runs: Vec<GqlCheckRun>, repo_url: &str) -> CiRun {
    let mut earliest_start: Option<u64> = None;
    let mut latest_completion: Option<u64> = None;

    let ci_jobs: Vec<CiJob> = check_runs
        .into_iter()
        .map(|job| {
            if let Some(start) = job.started_at.as_ref().and_then(|s| parse_iso8601(s).ok()) {
                earliest_start =
                    Some(earliest_start.map_or(start, |current: u64| current.min(start)));
            }
            if let Some(end) = job
                .completed_at
                .as_ref()
                .and_then(|s| parse_iso8601(s).ok())
            {
                latest_completion =
                    Some(latest_completion.map_or(end, |current: u64| current.max(end)));
            }
            let conclusion = parse_gql_conclusion(job.conclusion.as_deref());
            let duration_secs =
                compute_duration_secs(job.started_at.as_ref(), job.completed_at.as_ref());
            let duration = duration_secs.map_or_else(|| "—".to_string(), format_secs);
            CiJob {
                name: job.name,
                conclusion,
                duration,
                duration_secs,
            }
        })
        .collect();

    let wall_clock_secs = earliest_start
        .zip(latest_completion)
        .map(|(start, end)| end.saturating_sub(start));

    let conclusion = run_conclusion(&ci_jobs);

    CiRun {
        run_id: gh_run.id,
        created_at: gh_run.created_at.clone(),
        branch: gh_run.head_branch.clone(),
        url: format!("{repo_url}/actions/runs/{}", gh_run.id),
        conclusion,
        wall_clock_secs,
        jobs: ci_jobs,
        commit_title: gh_run.display_title.clone(),
        fetched: FetchStatus::Fetched,
    }
}

/// Fetch jobs for multiple runs in a single GraphQL call using node
/// ID aliases. Returns a map from `run_id` → check runs.
pub fn batch_fetch_jobs(repo_dir: &Path, runs: &[&GhRun]) -> HashMap<u64, Vec<GqlCheckRun>> {
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
                "run_{i}: node(id: \"{}\") {{ ... on WorkflowRun {{ databaseId {fragment} }} }}",
                run.node_id
            )
        })
        .collect();

    let query = format!("{{ {} }}", aliases.join(" "));
    let stdout = gh_command_with_timeout(
        repo_dir,
        &["api", "graphql", "-f", &format!("query={query}")],
    );

    let Some(stdout) = stdout else {
        return HashMap::new();
    };
    let Ok(response) = serde_json::from_slice::<GqlBatchResponse>(&stdout) else {
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

fn gh_command_with_timeout(repo_dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new("gh")
        .current_dir(repo_dir)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let output = child.wait_with_output().ok()?;
                return Some(output.stdout);
            },
            Ok(None) => {
                if start.elapsed() > GH_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            },
            Err(_) => return None,
        }
    }
}

/// Extract `(owner, repo)` from a GitHub URL like `https://github.com/owner/repo`.
pub fn parse_owner_repo(url: &str) -> Option<(String, String)> {
    let stripped = url.strip_prefix("https://github.com/")?;
    let mut parts = stripped.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

pub fn get_repo_url(repo_dir: &Path) -> Option<String> {
    let stdout = gh_command_with_timeout(repo_dir, &["repo", "view", "--json", "url"])?;
    let repo: GhRepo = serde_json::from_slice(&stdout).ok()?;
    Some(repo.url)
}

pub fn list_runs(repo_dir: &Path, branch: Option<&String>, count: u32) -> Option<Vec<GhRun>> {
    let mut endpoint =
        format!("repos/{{owner}}/{{repo}}/actions/runs?per_page={count}&status=completed");
    if let Some(branch) = branch {
        let _ = write!(endpoint, "&branch={branch}");
    }
    let stdout = gh_command_with_timeout(repo_dir, &["api", &endpoint])?;
    let response: GhRunsResponse = serde_json::from_slice(&stdout).ok()?;
    Some(response.workflow_runs)
}

fn run_conclusion(jobs: &[CiJob]) -> Conclusion {
    if jobs.iter().any(|j| j.conclusion.is_failure()) {
        return Conclusion::Failure;
    }
    if jobs.iter().any(|j| j.conclusion == Conclusion::Cancelled) {
        return Conclusion::Cancelled;
    }
    Conclusion::Success
}

/// Parse conclusion from GraphQL `CheckRun` (`SUCCESS`, `FAILURE`,
/// `CANCELLED`).
fn parse_gql_conclusion(conclusion: Option<&str>) -> Conclusion {
    match conclusion {
        Some("SUCCESS") => Conclusion::Success,
        Some("FAILURE") => Conclusion::Failure,
        _ => Conclusion::Cancelled,
    }
}

fn compute_duration_secs(
    started_at: Option<&String>,
    completed_at: Option<&String>,
) -> Option<u64> {
    let start = started_at?;
    let end = completed_at?;
    let start_ts = parse_iso8601(start).ok()?;
    let end_ts = parse_iso8601(end).ok()?;
    Some(end_ts.saturating_sub(start_ts))
}

pub fn format_secs(secs: u64) -> String {
    if secs >= 3600 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m:>2}m")
    } else if secs >= 60 {
        format!("{:>2}m {:>2}s", secs / 60, secs % 60)
    } else {
        format!("{secs:>2}s")
    }
}

/// Parses a subset of ISO 8601 timestamps (e.g. `2026-03-15T10:30:00Z`) into Unix seconds.
fn parse_iso8601(s: &str) -> Result<u64, ()> {
    // Expected format: YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim_end_matches('Z');
    let (date_part, time_part) = s.split_once('T').ok_or(())?;
    let date_parts: Vec<&str> = date_part.split('-').collect();
    let time_parts: Vec<&str> = time_part.split(':').collect();

    if date_parts.len() != 3 || time_parts.len() != 3 {
        return Err(());
    }

    let year: u64 = date_parts[0].parse().map_err(|_| ())?;
    let month: u64 = date_parts[1].parse().map_err(|_| ())?;
    let day: u64 = date_parts[2].parse().map_err(|_| ())?;
    let hour: u64 = time_parts[0].parse().map_err(|_| ())?;
    let min: u64 = time_parts[1].parse().map_err(|_| ())?;
    let sec: u64 = time_parts[2].parse().map_err(|_| ())?;

    // Days from year 0 to Unix epoch (1970-01-01)
    let days = days_from_civil(year, month, day);
    Ok(days * 86400 + hour * 3600 + min * 60 + sec)
}

/// Converts a civil date to days since epoch, using the algorithm from
/// Howard Hinnant's date library.
const fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch_0 = era * 146_097 + doe;
    // Shift: civil day 0 is 0000-03-01, Unix epoch is 1970-01-01 = day 719468
    days_since_epoch_0 - 719_468
}
