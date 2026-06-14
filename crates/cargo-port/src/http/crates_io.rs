use serde_json::Value;

use super::client::HttpClient;
use super::client::HttpOutcome;
use super::client::ServiceKind;
use super::client::ServiceSignal;
use super::constants::CRATES_IO_CRATE_KEY;
use super::constants::CRATES_IO_DOWNLOADS_KEY;
use super::constants::CRATES_IO_MAX_STABLE_VERSION_KEY;
use super::constants::CRATES_IO_MAX_VERSION_KEY;
use super::constants::USER_AGENT_HEADER;
use super::rate_limit;
use crate::constants::CRATES_IO_API_BASE;
use crate::constants::CRATES_IO_USER_AGENT;
use crate::scan::CratesIoInfo;

impl HttpClient {
    pub(crate) async fn fetch_crates_io_info_async(
        &self,
        crate_name: &str,
    ) -> HttpOutcome<CratesIoInfo> {
        let url = format!("{CRATES_IO_API_BASE}/crates/{crate_name}");
        let response = match self
            .client
            .get(&url)
            .header(USER_AGENT_HEADER, CRATES_IO_USER_AGENT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::CratesIo, &error),
                );
            },
        };
        // Surface a 429 as a rate-limit signal instead of a silent miss:
        // the service state machine pauses, probes, and refetches the
        // missing versions on recovery. Without this the body parse below
        // yields no version while reporting the service reachable.
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return (
                None,
                Some(ServiceSignal::RateLimited(ServiceKind::CratesIo)),
            );
        }
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                return (
                    None,
                    rate_limit::classify_network_error(ServiceKind::CratesIo, &error)
                        .or(Some(ServiceSignal::Reachable(ServiceKind::CratesIo))),
                );
            },
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        let Some(krate) = json.get(CRATES_IO_CRATE_KEY) else {
            return (None, Some(ServiceSignal::Reachable(ServiceKind::CratesIo)));
        };
        (
            crates_io_info_from_crate(krate),
            Some(ServiceSignal::Reachable(ServiceKind::CratesIo)),
        )
    }

    /// Fetch crates.io info (sync wrapper).
    pub(crate) fn fetch_crates_io_info(&self, crate_name: &str) -> HttpOutcome<CratesIoInfo> {
        self.handle
            .block_on(self.fetch_crates_io_info_async(crate_name))
    }
}

/// Select the version to show and (when distinct) the newer prerelease
/// from a crates.io `crate` object. `max_stable_version` is the latest
/// stable; `max_version` is the highest non-yanked release including
/// prereleases, so when it differs from the stable it must be a newer
/// prerelease. A crate with only prereleases shows the newest as its
/// version. Returns `None` when neither field is present.
fn crates_io_info_from_crate(krate: &Value) -> Option<CratesIoInfo> {
    let stable = krate
        .get(CRATES_IO_MAX_STABLE_VERSION_KEY)
        .and_then(serde_json::Value::as_str);
    let newest = krate
        .get(CRATES_IO_MAX_VERSION_KEY)
        .and_then(serde_json::Value::as_str);
    let (version, prerelease) = match (stable, newest) {
        (Some(stable), Some(newest)) if newest != stable && newest.contains('-') => {
            (stable.to_string(), Some(newest.to_string()))
        },
        (Some(stable), _) => (stable.to_string(), None),
        (None, Some(newest)) => (newest.to_string(), None),
        (None, None) => return None,
    };
    let downloads = krate
        .get(CRATES_IO_DOWNLOADS_KEY)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Some(CratesIoInfo {
        version,
        prerelease,
        downloads,
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod crates_io_tests {
    use super::crates_io_info_from_crate;

    #[test]
    fn stable_with_newer_prerelease_returns_both() {
        let krate = serde_json::json!({
            "max_stable_version": "0.20.2",
            "max_version": "0.21.0-rc.2",
            "downloads": 663,
        });
        let info = crates_io_info_from_crate(&krate).expect("info");
        assert_eq!(info.version, "0.20.2");
        assert_eq!(info.prerelease.as_deref(), Some("0.21.0-rc.2"));
        assert_eq!(info.downloads, 663);
    }

    #[test]
    fn stable_without_newer_prerelease_omits_prerelease() {
        let krate = serde_json::json!({
            "max_stable_version": "1.2.3",
            "max_version": "1.2.3",
            "downloads": 10,
        });
        let info = crates_io_info_from_crate(&krate).expect("info");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(info.prerelease, None);
    }

    #[test]
    fn only_prereleases_shows_newest_as_version() {
        let krate = serde_json::json!({
            "max_stable_version": serde_json::Value::Null,
            "max_version": "0.1.0-alpha.1",
            "downloads": 5,
        });
        let info = crates_io_info_from_crate(&krate).expect("info");
        assert_eq!(info.version, "0.1.0-alpha.1");
        assert_eq!(info.prerelease, None);
    }

    #[test]
    fn no_versions_returns_none() {
        let krate = serde_json::json!({ "downloads": 0 });
        assert!(crates_io_info_from_crate(&krate).is_none());
    }
}
