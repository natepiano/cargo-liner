use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use reqwest::Error;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::Value;

use super::ServiceKind;
use super::ServiceSignal;

/// Which GitHub rate-limit bucket a response belongs to. The REST and
/// GraphQL APIs share `api.github.com` but track their quotas
/// independently, so detection and display must keep them separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateLimitBucket {
    Core,
    GraphQl,
}

/// a single rate-limit bucket. `reset_at` is a Unix epoch
/// timestamp; `None` means the response did not include a reset header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RateLimitQuota {
    pub limit:     u64,
    pub used:      u64,
    pub remaining: u64,
    pub reset_at:  Option<u64>,
}

/// Live rate-limit state for both REST and GraphQL buckets. Either
/// field is `None` until a real response or `/rate_limit` poll
/// populates it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GitHubRateLimit {
    pub core:    Option<RateLimitQuota>,
    pub graphql: Option<RateLimitQuota>,
}

/// Read `X-RateLimit-*` headers off a GitHub response and identify which
/// bucket the response counted against. Returns `None` if the bucket
/// header is missing or names a resource we don't track (`search`,
/// `integration_manifest`, etc.).
pub(crate) fn parse_rate_limit_headers(
    headers: &HeaderMap,
) -> Option<(RateLimitBucket, RateLimitQuota)> {
    let resource = headers.get("x-ratelimit-resource")?.to_str().ok()?;
    let bucket = match resource {
        "core" => RateLimitBucket::Core,
        "graphql" => RateLimitBucket::GraphQl,
        _ => return None,
    };
    let parse = |name: &str| -> Option<u64> { headers.get(name)?.to_str().ok()?.parse().ok() };
    let limit = parse("x-ratelimit-limit")?;
    let used = parse("x-ratelimit-used")?;
    let remaining = parse("x-ratelimit-remaining")?;
    let reset_at = parse("x-ratelimit-reset");
    Some((
        bucket,
        RateLimitQuota {
            limit,
            used,
            remaining,
            reset_at,
        },
    ))
}

/// Parse a `/rate_limit` JSON response. Missing buckets stay `None` so
/// the caller can merge selectively.
pub(crate) fn parse_rate_limit_response(value: &Value) -> GitHubRateLimit {
    let resources = value.get("resources");
    let bucket = |name: &str| -> Option<RateLimitQuota> {
        let entry = resources?.get(name)?;
        Some(RateLimitQuota {
            limit:     entry.get("limit")?.as_u64()?,
            used:      entry.get("used")?.as_u64()?,
            remaining: entry.get("remaining")?.as_u64()?,
            reset_at:  entry.get("reset").and_then(serde_json::Value::as_u64),
        })
    };
    GitHubRateLimit {
        core:    bucket("core"),
        graphql: bucket("graphql"),
    }
}

/// True for the two REST forms GitHub uses for rate-limit refusals:
/// `429 Too Many Requests`, or `403 Forbidden` with
/// `X-RateLimit-Remaining: 0` (the secondary-rate-limit / abuse-detection
/// form). A bare 403 is auth-related and not rate-limit.
pub(crate) fn github_is_rate_limited(status: StatusCode, headers: &HeaderMap) -> bool {
    if status.as_u16() == 429 {
        return true;
    }
    if status.as_u16() == 403 {
        return headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<u64>().ok())
            .is_some_and(|remaining| remaining == 0);
    }
    false
}

/// True when a GraphQL response body carries an `errors[].type` of
/// `RATE_LIMITED`. GraphQL returns HTTP 200 on rate-limit, so
/// status-based detection alone is not enough for that endpoint.
pub(crate) fn graphql_body_is_rate_limited(body: &Value) -> bool {
    body.get("errors")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|errors| {
            errors.iter().any(|err| {
                err.get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|t| t == "RATE_LIMITED")
            })
        })
}

pub(super) fn classify_network_error(service: ServiceKind, error: &Error) -> Option<ServiceSignal> {
    if error.is_connect() || error.is_timeout() {
        Some(ServiceSignal::Unreachable(service))
    } else {
        None
    }
}

/// Lead time for the synthetic force-rate-limit countdown. `3599`
/// rather than `3600` so the first displayed value is `00:59:59`
/// instead of briefly flashing `01:00:00`.
pub(super) const SYNTHETIC_RATE_LIMIT_SECS: u64 = 3599;

pub(super) fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
