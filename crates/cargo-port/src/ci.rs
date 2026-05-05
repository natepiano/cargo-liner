use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use super::constants::CANCELLED;
use super::constants::FAILING;
use super::constants::PASSING;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct OwnerRepo {
    owner: String,
    repo:  String,
}

impl OwnerRepo {
    pub(crate) fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo:  repo.into(),
        }
    }

    pub(crate) fn owner(&self) -> &str { &self.owner }

    pub(crate) fn repo(&self) -> &str { &self.repo }
}

impl Display for OwnerRepo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

/// Workflow run from the GitHub REST API (`/actions/runs`).
#[derive(Deserialize)]
pub(crate) struct GhRun {
    pub id:            u64,
    pub node_id:       String,
    pub created_at:    String,
    pub updated_at:    String,
    pub head_branch:   String,
    pub display_title: Option<String>,
}

/// Job from the GraphQL `checkRuns` response (upper-case conclusion).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GqlCheckRun {
    pub(super) name:         String,
    pub(super) conclusion:   Option<String>,
    pub(super) started_at:   Option<String>,
    pub(super) completed_at: Option<String>,
}

/// Whether a CI run has been fully fetched from the API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FetchStatus {
    #[default]
    Fetched,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Conclusion {
    Success,
    Failure,
    Cancelled,
}

impl Conclusion {
    pub(crate) const fn icon(self) -> &'static str {
        match self {
            Self::Success => PASSING,
            Self::Failure => FAILING,
            Self::Cancelled => CANCELLED,
        }
    }

    pub(crate) const fn is_success(self) -> bool { matches!(self, Self::Success) }

    pub(crate) const fn is_failure(self) -> bool { matches!(self, Self::Failure) }
}

impl Display for Conclusion {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { f.write_str(self.icon()) }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CiRun {
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
    pub updated_at:      Option<String>,
    #[serde(default)]
    pub fetched:         FetchStatus,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CiJob {
    pub name:          String,
    pub conclusion:    Conclusion,
    pub duration:      String,
    pub duration_secs: Option<u64>,
}

/// Build a `CiRun` from a `GhRun` and pre-fetched check run data.
pub(crate) fn build_ci_run(gh_run: &GhRun, check_runs: Vec<GqlCheckRun>, repo_url: &str) -> CiRun {
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
        updated_at: Some(gh_run.updated_at.clone()),
        fetched: FetchStatus::Fetched,
    }
}

/// Extract `owner/repo` from a GitHub URL like `https://github.com/owner/repo`.
pub(crate) fn parse_owner_repo(url: &str) -> Option<OwnerRepo> {
    let stripped = url.strip_prefix("https://github.com/")?;
    let mut parts = stripped.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(OwnerRepo::new(owner, repo))
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

pub(crate) fn format_secs(secs: u64) -> String {
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
