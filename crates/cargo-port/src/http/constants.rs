// GitHub rate limits
pub(super) const GITHUB_CORE_BUCKET: &str = "core";
pub(super) const GITHUB_GRAPHQL_BUCKET: &str = "graphql";
pub(super) const GRAPHQL_RATE_LIMITED_ERROR_TYPE: &str = "RATE_LIMITED";
pub(super) const RATE_LIMIT_LIMIT_HEADER: &str = "x-ratelimit-limit";
pub(super) const RATE_LIMIT_REMAINING_HEADER: &str = "x-ratelimit-remaining";
pub(super) const RATE_LIMIT_RESET_HEADER: &str = "x-ratelimit-reset";
pub(super) const RATE_LIMIT_RESOURCE_HEADER: &str = "x-ratelimit-resource";
pub(super) const RATE_LIMIT_USED_HEADER: &str = "x-ratelimit-used";

// http headers
pub(super) const ACCEPT_HEADER: &str = "Accept";
pub(super) const AUTHORIZATION_HEADER: &str = "Authorization";
pub(super) const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub(super) const USER_AGENT_HEADER: &str = "User-Agent";

// media types
pub(super) const GITHUB_JSON_MEDIA_TYPE: &str = "application/vnd.github+json";
pub(super) const JSON_MEDIA_TYPE: &str = "application/json";
