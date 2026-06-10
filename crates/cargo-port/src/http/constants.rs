// crates.io schema
pub(super) const CRATES_IO_CRATE_KEY: &str = "crate";
pub(super) const CRATES_IO_DOWNLOADS_KEY: &str = "downloads";
pub(super) const CRATES_IO_MAX_STABLE_VERSION_KEY: &str = "max_stable_version";
pub(super) const CRATES_IO_MAX_VERSION_KEY: &str = "max_version";

// GitHub GraphQL schema
pub(super) const GITHUB_GRAPHQL_DATA_KEY: &str = "data";
pub(super) const GITHUB_GRAPHQL_DESCRIPTION_KEY: &str = "description";
pub(super) const GITHUB_GRAPHQL_REPO_KEY: &str = "repo";
pub(super) const GITHUB_GRAPHQL_RUN_ALIAS_PREFIX: &str = "run_";
pub(super) const GITHUB_GRAPHQL_STARGAZER_COUNT_KEY: &str = "stargazerCount";

// GitHub rate limits
pub(super) const GITHUB_CORE_BUCKET: &str = "core";
pub(super) const GITHUB_CORE_RATE_LIMIT_CAP: u64 = 5000;
pub(super) const GITHUB_GRAPHQL_BUCKET: &str = "graphql";
pub(super) const GITHUB_PR_PAGE_CAP: usize = 20;
pub(super) const GITHUB_PR_PAGE_SIZE: usize = 50;
pub(super) const GRAPHQL_RATE_LIMITED_ERROR_TYPE: &str = "RATE_LIMITED";
pub(super) const GRAPHQL_RESPONSE_ERRORS_KEY: &str = "errors";
pub(super) const GRAPHQL_RESPONSE_TYPE_KEY: &str = "type";
pub(super) const RATE_LIMIT_LIMIT_KEY: &str = "limit";
pub(super) const RATE_LIMIT_LIMIT_HEADER: &str = "x-ratelimit-limit";
pub(super) const RATE_LIMIT_REMAINING_KEY: &str = "remaining";
pub(super) const RATE_LIMIT_REMAINING_HEADER: &str = "x-ratelimit-remaining";
pub(super) const RATE_LIMIT_RESET_KEY: &str = "reset";
pub(super) const RATE_LIMIT_RESET_HEADER: &str = "x-ratelimit-reset";
pub(super) const RATE_LIMIT_RESOURCES_KEY: &str = "resources";
pub(super) const RATE_LIMIT_RESOURCE_HEADER: &str = "x-ratelimit-resource";
pub(super) const RATE_LIMIT_USED_KEY: &str = "used";
pub(super) const RATE_LIMIT_USED_HEADER: &str = "x-ratelimit-used";

// http headers
pub(super) const ACCEPT_HEADER: &str = "Accept";
pub(super) const AUTHORIZATION_HEADER: &str = "Authorization";
pub(super) const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub(super) const USER_AGENT_HEADER: &str = "User-Agent";

// media types
pub(super) const GITHUB_JSON_MEDIA_TYPE: &str = "application/vnd.github+json";
pub(super) const JSON_MEDIA_TYPE: &str = "application/json";
