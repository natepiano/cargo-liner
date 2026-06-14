use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::project::git::command;
use crate::project::git::constants::GIT_CONFIG_COMMAND;
use crate::project::git::constants::GIT_CONFIG_REMOTE_PREFIX;
use crate::project::git::constants::GIT_CONFIG_REMOTE_PUSHURL_PATTERN;
use crate::project::git::constants::GIT_CONFIG_REMOTE_PUSHURL_SUFFIX;
use crate::project::git::constants::GIT_GET_REGEXP_ARG;

/// A well-known push-disable sentinel that users put in `remote.<name>.pushurl`
/// to lock out accidental pushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnownSentinel {
    Disabled,
    NoPush,
    DoNotPush,
}

impl KnownSentinel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::NoPush => "no-push",
            Self::DoNotPush => "do_not_push",
        }
    }

    fn from_pushurl(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "disabled" => Some(Self::Disabled),
            "no-push" => Some(Self::NoPush),
            "do_not_push" => Some(Self::DoNotPush),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub(crate) enum PushDisabledReason {
    KnownSentinel(KnownSentinel),
    NoPushUrl,
}

/// Whether `git push` against this remote is enabled, and the URL it
/// would push to. Derived from `git config remote.<name>.pushurl` —
/// when unset, push resolves to the fetch URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub(crate) enum PushState {
    Enabled { push_url: String },
    Disabled { reason: PushDisabledReason },
}

/// Map `pushurl` (or its absence) and the remote's fetch URL into a
/// `PushState`. Rules:
///
/// - No `pushurl` entry → `Enabled` with the fetch URL.
/// - Empty `pushurl` → `Disabled { NoPushUrl }`.
/// - `pushurl` matches a known sentinel (case-insensitive) → `Disabled { KnownSentinel(_) }`.
/// - Any other `pushurl` → `Enabled` with that URL. Anything that looks intentionally non-routable
///   is not heuristically demoted to disabled in this stage — explicit sentinels only.
pub(super) fn resolve_push_state(fetch_url: Option<&str>, pushurl: Option<&str>) -> PushState {
    let push_url_for_fetch = || PushState::Enabled {
        push_url: fetch_url.unwrap_or_default().to_string(),
    };
    let Some(value) = pushurl else {
        return push_url_for_fetch();
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return PushState::Disabled {
            reason: PushDisabledReason::NoPushUrl,
        };
    }
    if let Some(sentinel) = KnownSentinel::from_pushurl(trimmed) {
        return PushState::Disabled {
            reason: PushDisabledReason::KnownSentinel(sentinel),
        };
    }
    PushState::Enabled {
        push_url: trimmed.to_string(),
    }
}

/// Batch-read every `remote.<name>.pushurl` value with a single
/// `git config --get-regexp` shell-out. Returns a map keyed by remote
/// name (with `remote.` and `.pushurl` stripped).
pub(super) fn list_remote_pushurls(repo_root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(output) = command::git_output_logged(
        repo_root,
        "config_get_regexp_pushurl",
        [
            GIT_CONFIG_COMMAND,
            GIT_GET_REGEXP_ARG,
            GIT_CONFIG_REMOTE_PUSHURL_PATTERN,
        ],
    ) else {
        return map;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(rest) = key.strip_prefix(GIT_CONFIG_REMOTE_PREFIX) else {
            continue;
        };
        let Some(name) = rest.strip_suffix(GIT_CONFIG_REMOTE_PUSHURL_SUFFIX) else {
            continue;
        };
        map.insert(name.to_string(), value.to_string());
    }
    map
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn push_state_unset_uses_fetch_url() {
        let push = resolve_push_state(Some("https://github.com/a/b.git"), None);
        assert_eq!(
            push,
            PushState::Enabled {
                push_url: "https://github.com/a/b.git".to_string(),
            }
        );
    }

    #[test]
    fn push_state_empty_is_no_push_url() {
        let push = resolve_push_state(Some("https://github.com/a/b.git"), Some(""));
        assert_eq!(
            push,
            PushState::Disabled {
                reason: PushDisabledReason::NoPushUrl,
            }
        );
    }

    #[test]
    fn push_state_disabled_sentinel_case_insensitive() {
        for value in ["DISABLED", "disabled", "Disabled"] {
            let push = resolve_push_state(Some("ignored"), Some(value));
            assert_eq!(
                push,
                PushState::Disabled {
                    reason: PushDisabledReason::KnownSentinel(KnownSentinel::Disabled),
                }
            );
        }
    }

    #[test]
    fn push_state_unknown_pushurl_stays_enabled() {
        let push = resolve_push_state(Some("https://github.com/a/b.git"), Some("ssh://other/repo"));
        assert_eq!(
            push,
            PushState::Enabled {
                push_url: "ssh://other/repo".to_string(),
            }
        );
    }

    #[test]
    fn push_state_serde_round_trip() {
        for state in [
            PushState::Enabled {
                push_url: "https://example.com".to_string(),
            },
            PushState::Disabled {
                reason: PushDisabledReason::NoPushUrl,
            },
            PushState::Disabled {
                reason: PushDisabledReason::KnownSentinel(KnownSentinel::Disabled),
            },
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let _: Value = serde_json::from_str(&json).expect("valid JSON");
        }
    }
}
