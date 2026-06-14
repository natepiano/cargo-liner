use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use super::client::HttpClient;
use super::client::HttpOutcome;
use super::client::ServiceKind;
use super::client::ServiceSignal;
use super::constants::GITHUB_PR_PAGE_CAP;
use super::constants::GITHUB_PR_PAGE_SIZE;
use crate::ci::OwnerRepo;
use crate::project::ProjectPrInfo;
use crate::project::PullRequestCompleteness;
use crate::project::PullRequestGoneReason;
use crate::project::PullRequestInfo;
use crate::project::PullRequestState;
use crate::project::PullRequestUnavailableReason;

pub(crate) enum PullRequestFetch {
    Loaded(ProjectPrInfo),
    Unavailable(PullRequestUnavailableReason),
}

type PullRequestPages = Result<
    (Vec<GqlPullRequestNode>, String, PullRequestCompleteness),
    PullRequestUnavailableReason,
>;

#[derive(Deserialize)]
struct GqlViewerResponse {
    data:   Option<GqlViewerData>,
    errors: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct GqlViewerData {
    viewer: GqlViewer,
}

#[derive(Deserialize)]
struct GqlViewer {
    login: String,
}

#[derive(Deserialize)]
struct GqlPullRequestsResponse {
    data:   Option<GqlPullRequestsData>,
    errors: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct GqlPullRequestsData {
    repository: Option<GqlPullRequestRepository>,
    search:     Option<GqlPullRequestSearch>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequestRepository {
    default_branch_ref: Option<GqlBranchRef>,
}

#[derive(Deserialize)]
struct GqlBranchRef {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequestSearch {
    page_info: GqlPageInfo,
    nodes:     Vec<Option<GqlPullRequestNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPageInfo {
    has_next_page: bool,
    end_cursor:    Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequestNode {
    number:             u32,
    title:              String,
    url:                String,
    is_draft:           bool,
    review_decision:    Option<String>,
    merge_state_status: Option<String>,
    head_ref_name:      String,
    base_ref_name:      String,
    head_repository:    Option<GqlPullRequestHeadRepository>,
}

#[derive(Deserialize)]
struct GqlPullRequestHeadRepository {
    name:  String,
    owner: GqlRepositoryOwner,
}

#[derive(Deserialize)]
struct GqlRepositoryOwner {
    login: String,
}

#[derive(Deserialize)]
struct GqlPullRequestStatusResponse {
    data:   Option<GqlPullRequestStatusData>,
    errors: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct GqlPullRequestStatusData {
    repository: Option<GqlPullRequestStatusRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequestStatusRepository {
    pull_request: Option<GqlPullRequestStatusNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequestStatusNode {
    merged:        bool,
    closed:        bool,
    base_ref_name: String,
}

const fn unavailable_reason_from_signal(
    signal: Option<ServiceSignal>,
) -> PullRequestUnavailableReason {
    match signal {
        Some(ServiceSignal::RateLimited(ServiceKind::GitHub)) => {
            PullRequestUnavailableReason::RateLimited
        },
        Some(ServiceSignal::Unreachable(ServiceKind::GitHub)) => {
            PullRequestUnavailableReason::Network
        },
        _ => PullRequestUnavailableReason::GraphQlError,
    }
}

const fn combine_optional_signal(
    left: Option<ServiceSignal>,
    right: Option<ServiceSignal>,
) -> Option<ServiceSignal> {
    match (left, right) {
        (Some(ServiceSignal::Unreachable(service)), _)
        | (_, Some(ServiceSignal::Unreachable(service))) => {
            Some(ServiceSignal::Unreachable(service))
        },
        (Some(ServiceSignal::RateLimited(service)), _)
        | (_, Some(ServiceSignal::RateLimited(service))) => {
            Some(ServiceSignal::RateLimited(service))
        },
        (Some(ServiceSignal::Reachable(service)), _)
        | (_, Some(ServiceSignal::Reachable(service))) => Some(ServiceSignal::Reachable(service)),
        (None, None) => None,
    }
}

fn graphql_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn pull_requests_query(
    owner: &str,
    repo: &str,
    search_query: &str,
    cursor: Option<&str>,
) -> String {
    let owner = graphql_string(owner);
    let repo = graphql_string(repo);
    let search_query = graphql_string(search_query);
    let after = cursor.map_or_else(String::new, |cursor| {
        format!(", after: {}", graphql_string(cursor))
    });
    format!(
        "{{ repository(owner: {owner}, name: {repo}) {{ defaultBranchRef {{ name }} }} \
         search(type: ISSUE, first: {GITHUB_PR_PAGE_SIZE}{after}, query: {search_query}) \
         {{ pageInfo {{ hasNextPage endCursor }} nodes {{ ... on PullRequest {{ number title url \
         isDraft reviewDecision mergeStateStatus headRefName baseRefName \
         headRepository {{ name owner {{ login }} }} }} }} }} }}"
    )
}

fn pull_request_status_query(owner: &str, repo: &str, number: u32) -> String {
    let owner = graphql_string(owner);
    let repo = graphql_string(repo);
    format!(
        "{{ repository(owner: {owner}, name: {repo}) {{ pullRequest(number: {number}) \
         {{ merged closed baseRefName }} }} }}"
    )
}

fn build_project_pr_info(
    owner_repo: OwnerRepo,
    viewer_login: String,
    default_branch: String,
    nodes: Vec<GqlPullRequestNode>,
    completeness: PullRequestCompleteness,
) -> ProjectPrInfo {
    ProjectPrInfo {
        open: nodes.into_iter().map(pull_request_info_from_node).collect(),
        default_branch,
        fetched_at: Utc::now().format("%+").to_string(),
        completeness,
        viewer_login,
        owner_repo,
    }
}

fn pull_request_info_from_node(node: GqlPullRequestNode) -> PullRequestInfo {
    let state = reduce_pull_request_state(
        node.is_draft,
        node.review_decision.as_deref(),
        node.merge_state_status.as_deref(),
    );
    PullRequestInfo {
        number: node.number,
        title: node.title,
        url: node.url,
        state,
        head: node.head_ref_name,
        head_owner: node
            .head_repository
            .as_ref()
            .map(|repo| repo.owner.login.clone()),
        head_repo: node.head_repository.map(|repo| repo.name),
        base: node.base_ref_name,
    }
}

fn reduce_pull_request_state(
    is_draft: bool,
    review_decision: Option<&str>,
    merge_state_status: Option<&str>,
) -> PullRequestState {
    if is_draft {
        return PullRequestState::Draft;
    }
    if review_decision == Some("CHANGES_REQUESTED") {
        return PullRequestState::ChangesRequested;
    }
    match merge_state_status {
        Some("UNSTABLE") => return PullRequestState::ChecksFailing,
        Some("BLOCKED" | "DIRTY" | "HAS_HOOKS") => return PullRequestState::Blocked,
        Some("BEHIND") => return PullRequestState::Behind,
        _ => {},
    }
    match review_decision {
        Some("REVIEW_REQUIRED") => PullRequestState::ReviewRequired,
        Some("APPROVED") => PullRequestState::Approved,
        _ => match merge_state_status {
            Some("CLEAN") | None => PullRequestState::Ready,
            Some(_) => PullRequestState::Unknown,
        },
    }
}

impl HttpClient {
    async fn github_viewer_login_async(
        &self,
    ) -> HttpOutcome<Result<String, PullRequestUnavailableReason>> {
        if let Ok(cache) = self.github_viewer_login.lock()
            && let Some(login) = cache.clone()
        {
            return (Some(Ok(login)), None);
        }
        let (body, signal) = self.github_graphql_async("{ viewer { login } }").await;
        let Some(body) = body else {
            return (Some(Err(unavailable_reason_from_signal(signal))), signal);
        };
        let Ok(response) = serde_json::from_slice::<GqlViewerResponse>(&body) else {
            return (
                Some(Err(PullRequestUnavailableReason::GraphQlError)),
                signal,
            );
        };
        if response
            .errors
            .as_ref()
            .is_some_and(|errors| !errors.is_empty())
        {
            return (Some(Err(PullRequestUnavailableReason::Forbidden)), signal);
        }
        let Some(login) = response.data.map(|data| data.viewer.login) else {
            return (
                Some(Err(PullRequestUnavailableReason::GraphQlError)),
                signal,
            );
        };
        if let Ok(mut cache) = self.github_viewer_login.lock() {
            *cache = Some(login.clone());
        }
        (Some(Ok(login)), signal)
    }

    pub(crate) async fn fetch_open_pull_requests_async(
        &self,
        owner_repo: OwnerRepo,
    ) -> HttpOutcome<PullRequestFetch> {
        if !self.has_github_token() {
            return (
                Some(PullRequestFetch::Unavailable(
                    PullRequestUnavailableReason::Unauthenticated,
                )),
                None,
            );
        }
        if self
            .rate_limit()
            .graphql
            .is_some_and(|quota| quota.remaining == 0)
        {
            return (
                Some(PullRequestFetch::Unavailable(
                    PullRequestUnavailableReason::RateLimited,
                )),
                Some(ServiceSignal::RateLimited(ServiceKind::GitHub)),
            );
        }

        let (viewer, viewer_signal) = self.github_viewer_login_async().await;
        let Some(Ok(viewer_login)) = viewer else {
            let reason = viewer
                .and_then(Result::err)
                .unwrap_or_else(|| unavailable_reason_from_signal(viewer_signal));
            return (Some(PullRequestFetch::Unavailable(reason)), viewer_signal);
        };

        let search_query = format!(
            "repo:{}/{} is:pr is:open author:{viewer_login}",
            owner_repo.owner(),
            owner_repo.repo()
        );
        let (pages, signal) = self
            .fetch_pull_request_pages(&owner_repo, &search_query)
            .await;
        let Some(Ok((nodes, default_branch, completeness))) = pages else {
            let reason = pages
                .and_then(Result::err)
                .unwrap_or_else(|| unavailable_reason_from_signal(signal));
            return (Some(PullRequestFetch::Unavailable(reason)), signal);
        };
        let info = build_project_pr_info(
            owner_repo,
            viewer_login,
            default_branch,
            nodes,
            completeness,
        );
        (Some(PullRequestFetch::Loaded(info)), signal)
    }

    pub(crate) async fn fetch_pull_request_gone_reason_async(
        &self,
        owner_repo: OwnerRepo,
        number: u32,
    ) -> HttpOutcome<PullRequestGoneReason> {
        if !self.has_github_token() {
            return (Some(PullRequestGoneReason::Unknown), None);
        }
        let query = pull_request_status_query(owner_repo.owner(), owner_repo.repo(), number);
        let (body, signal) = self.github_graphql_async(&query).await;
        let Some(body) = body else {
            return (Some(PullRequestGoneReason::Unknown), signal);
        };
        let Ok(response) = serde_json::from_slice::<GqlPullRequestStatusResponse>(&body) else {
            return (Some(PullRequestGoneReason::Unknown), signal);
        };
        if response
            .errors
            .as_ref()
            .is_some_and(|errors| !errors.is_empty())
        {
            return (Some(PullRequestGoneReason::Unknown), signal);
        }
        let Some(repository) = response.data.and_then(|data| data.repository) else {
            return (Some(PullRequestGoneReason::Missing), signal);
        };
        let Some(pull_request) = repository.pull_request else {
            return (Some(PullRequestGoneReason::Missing), signal);
        };
        let reason = if pull_request.merged {
            PullRequestGoneReason::Merged {
                base: pull_request.base_ref_name,
            }
        } else if pull_request.closed {
            PullRequestGoneReason::Closed
        } else {
            PullRequestGoneReason::Unknown
        };
        (Some(reason), signal)
    }

    async fn fetch_pull_request_pages(
        &self,
        owner_repo: &OwnerRepo,
        search_query: &str,
    ) -> HttpOutcome<PullRequestPages> {
        let mut all_nodes = Vec::new();
        let mut cursor: Option<String> = None;
        let mut default_branch = None;
        let mut signal = None;

        for _ in 0..GITHUB_PR_PAGE_CAP {
            let query = pull_requests_query(
                owner_repo.owner(),
                owner_repo.repo(),
                search_query,
                cursor.as_deref(),
            );
            let (body, page_signal) = self.github_graphql_async(&query).await;
            signal = combine_optional_signal(signal, page_signal);
            let Some(body) = body else {
                return (Some(Err(unavailable_reason_from_signal(signal))), signal);
            };
            let Ok(response) = serde_json::from_slice::<GqlPullRequestsResponse>(&body) else {
                return (
                    Some(Err(PullRequestUnavailableReason::GraphQlError)),
                    signal,
                );
            };
            if response
                .errors
                .as_ref()
                .is_some_and(|errors| !errors.is_empty())
            {
                return (
                    Some(Err(PullRequestUnavailableReason::GraphQlError)),
                    signal,
                );
            }
            let Some(data) = response.data else {
                return (
                    Some(Err(PullRequestUnavailableReason::GraphQlError)),
                    signal,
                );
            };
            let Some(repository) = data.repository else {
                return (
                    Some(Err(PullRequestUnavailableReason::RepositoryMissing)),
                    signal,
                );
            };
            if default_branch.is_none() {
                default_branch = repository.default_branch_ref.map(|branch| branch.name);
            }
            let Some(search) = data.search else {
                return (
                    Some(Err(PullRequestUnavailableReason::GraphQlError)),
                    signal,
                );
            };
            all_nodes.extend(search.nodes.into_iter().flatten());
            if !search.page_info.has_next_page {
                return (
                    Some(Ok((
                        all_nodes,
                        default_branch.unwrap_or_else(|| "main".to_string()),
                        PullRequestCompleteness::Complete,
                    ))),
                    signal,
                );
            }
            let Some(next_cursor) = search.page_info.end_cursor else {
                return (
                    Some(Err(PullRequestUnavailableReason::IncompletePagination)),
                    signal,
                );
            };
            cursor = Some(next_cursor);
        }

        let shown = all_nodes.len();
        (
            Some(Ok((
                all_nodes,
                default_branch.unwrap_or_else(|| "main".to_string()),
                PullRequestCompleteness::Truncated { shown },
            ))),
            signal,
        )
    }

    /// Call GitHub's `/rate_limit` endpoint, which is itself exempt from
    /// the quota and therefore safe to poll while we're rate-limited.
    /// Updates the shared live `rate_limit` on success.
    pub(crate) fn fetch_open_pull_requests(
        &self,
        owner_repo: OwnerRepo,
    ) -> HttpOutcome<PullRequestFetch> {
        self.handle
            .block_on(self.fetch_open_pull_requests_async(owner_repo))
    }

    pub(crate) fn fetch_pull_request_gone_reason(
        &self,
        owner_repo: OwnerRepo,
        number: u32,
    ) -> HttpOutcome<PullRequestGoneReason> {
        self.handle
            .block_on(self.fetch_pull_request_gone_reason_async(owner_repo, number))
    }
}
