//! The branch selected for a reservation when it is acquired.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::FullRefName;

/// A local branch into which reserved work integrates.
#[derive(Clone, Debug, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct IntegrationTarget(FullRefName);

impl<'de> Deserialize<'de> for IntegrationTarget {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let reference = String::deserialize(deserializer)?;
        if !reference.starts_with("refs/heads/") {
            return Err(serde::de::Error::custom(
                "integration target must be a local branch ref",
            ));
        }
        Self::from_branch_argument(&reference).map_err(serde::de::Error::custom)
    }
}

impl IntegrationTarget {
    /// Parse a short local branch name or its complete local ref.
    pub(crate) fn from_branch_argument(value: &str) -> Result<Self, String> {
        let branch = value.strip_prefix("refs/heads/").unwrap_or(value);
        if branch.starts_with("refs/") || branch.is_empty() {
            return Err(format!("target `{value}` is not a local branch"));
        }
        let reference = FullRefName::from_str(&format!("refs/heads/{branch}"))
            .map_err(|_| format!("target `{value}` is not a valid local branch"))?;
        Ok(Self(reference))
    }

    /// The complete local ref.
    pub(crate) const fn reference(&self) -> &FullRefName { &self.0 }

    /// The local branch name without `refs/heads/`.
    pub(crate) fn short_name(&self) -> &str {
        self.0
            .as_str()
            .strip_prefix("refs/heads/")
            .unwrap_or_default()
    }
}

/// The input that selected a claim's integration branch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetSource {
    /// An explicit `claim --target` or `retarget --target` argument.
    ClaimArgument,
    /// The claimant's `branch.<name>.cargoBerthTarget` setting.
    BranchConfiguration,
    /// Repository policy supplied the default.
    RepositoryTrunk,
}

/// A target selected when a claim was appended.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct ClaimTarget {
    pub(crate) target:   IntegrationTarget,
    pub(crate) source:   TargetSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fallback: Option<TargetFallback>,
}

/// An invalid branch setting replaced by the repository trunk during automatic acquisition.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct TargetFallback {
    requested: String,
    reason:    TargetFallbackReason,
}

/// Why an automatically selected branch setting could not be used.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TargetFallbackReason {
    OwnBranch,
    Unresolved,
}

/// How a claim selects its integration branch.
#[derive(Clone, Copy)]
pub(crate) enum TargetSelectionRequest<'a> {
    ExplicitClaimWithArgument(&'a str),
    ExplicitClaimFromBranch,
    AutomaticAcquisition,
}

/// An explicit target that cannot be recorded.
#[derive(Debug)]
pub(crate) struct TargetRefusal {
    requested: String,
    reason:    TargetFallbackReason,
}

impl TargetRefusal {
    pub(crate) fn unresolved(requested: &str) -> Self {
        Self {
            requested: requested.to_owned(),
            reason:    TargetFallbackReason::Unresolved,
        }
    }

    pub(crate) fn own_branch(requested: &str) -> Self {
        Self {
            requested: requested.to_owned(),
            reason:    TargetFallbackReason::OwnBranch,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self.reason {
            TargetFallbackReason::OwnBranch => {
                format!("target `{}` is the claimant's own branch", self.requested)
            },
            TargetFallbackReason::Unresolved => format!(
                "target `{}` does not resolve to a local branch",
                self.requested
            ),
        }
    }
}

/// Read the last matching branch setting from the common git configuration file.
fn branch_target_setting(common_git_directory: &Path, branch: &str) -> Option<String> {
    let contents = fs::read_to_string(common_git_directory.join("config")).ok()?;
    branch_target_from_config_text(&contents, branch)
}

fn branch_target_from_config_text(contents: &str, branch: &str) -> Option<String> {
    let mut matching_section = false;
    let mut selected = None;
    for line in contents.lines() {
        let mut quoted = false;
        let mut escaped = false;
        let comment_start = line.char_indices().find_map(|(index, character)| {
            if escaped {
                escaped = false;
            } else if character == '\\' && quoted {
                escaped = true;
            } else if character == '"' {
                quoted = !quoted;
            } else if !quoted && matches!(character, '#' | ';') {
                return Some(index);
            }
            None
        });
        let line = line[..comment_start.unwrap_or(line.len())].trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            matching_section = section
                .strip_prefix("branch \"")
                .and_then(|subsection| subsection.strip_suffix('"'))
                .is_some_and(|subsection| subsection == branch);
            continue;
        }
        if matching_section
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("cargoBerthTarget")
        {
            selected = Some(value.trim().trim_matches('"').to_owned());
        }
    }
    selected
}

/// Select a branch, checking that it differs from the claimant and resolves locally.
pub(crate) fn resolve_claim_target(
    common_git_directory: &Path,
    claimant_branch: Option<&FullRefName>,
    request: TargetSelectionRequest<'_>,
    repository_trunk: &IntegrationTarget,
    resolves: impl Fn(&IntegrationTarget) -> bool,
) -> Result<ClaimTarget, TargetRefusal> {
    let setting = match request {
        TargetSelectionRequest::ExplicitClaimWithArgument(_) => None,
        TargetSelectionRequest::ExplicitClaimFromBranch
        | TargetSelectionRequest::AutomaticAcquisition => claimant_branch.and_then(|branch| {
            branch_target_setting(
                common_git_directory,
                branch.as_str().trim_start_matches("refs/heads/"),
            )
        }),
    };
    let (requested, source) =
        if let TargetSelectionRequest::ExplicitClaimWithArgument(argument) = request {
            (argument.to_owned(), TargetSource::ClaimArgument)
        } else if let Some(setting) = setting {
            (setting, TargetSource::BranchConfiguration)
        } else {
            return Ok(ClaimTarget {
                target:   repository_trunk.clone(),
                source:   TargetSource::RepositoryTrunk,
                fallback: None,
            });
        };
    let target = IntegrationTarget::from_branch_argument(&requested);
    let reason = match &target {
        Ok(target) if claimant_branch == Some(target.reference()) => {
            Some(TargetFallbackReason::OwnBranch)
        },
        Ok(target) if !resolves(target) => Some(TargetFallbackReason::Unresolved),
        Ok(_) => None,
        Err(_) => Some(TargetFallbackReason::Unresolved),
    };
    match reason {
        Some(reason) if matches!(request, TargetSelectionRequest::AutomaticAcquisition) => {
            Ok(ClaimTarget {
                target:   repository_trunk.clone(),
                source:   TargetSource::RepositoryTrunk,
                fallback: Some(TargetFallback { requested, reason }),
            })
        },
        Some(reason) => Err(TargetRefusal { requested, reason }),
        None => target
            .map_err(|_| TargetRefusal::unresolved(&requested))
            .map(|target| ClaimTarget {
                target,
                source,
                fallback: None,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::IntegrationTarget;
    use super::TargetSelectionRequest;
    use super::branch_target_from_config_text;
    use super::resolve_claim_target;

    #[test]
    fn branch_setting_ignores_comments_outside_quotes() {
        let config =
            "[branch \"lane\"] # section note\n cargoBerthTarget = integration # value note\n";
        assert_eq!(
            branch_target_from_config_text(config, "lane").as_deref(),
            Some("integration")
        );
        let config =
            "[branch \"lane\"] ; section note\n cargoBerthTarget = \"feat#1\" ; value note\n";
        assert_eq!(
            branch_target_from_config_text(config, "lane").as_deref(),
            Some("feat#1")
        );
    }

    #[test]
    fn explicit_target_refuses_invalid_setting_while_automatic_acquisition_falls_back()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        std::fs::write(
            directory.path().join("config"),
            "[branch \"lane\"]\n cargoBerthTarget = missing\n",
        )?;
        let claimant = "refs/heads/lane"
            .parse()
            .map_err(|_| std::io::Error::other("branch ref"))?;
        let trunk =
            IntegrationTarget::from_branch_argument("main").map_err(std::io::Error::other)?;
        let Err(refusal) = resolve_claim_target(
            directory.path(),
            Some(&claimant),
            TargetSelectionRequest::ExplicitClaimFromBranch,
            &trunk,
            |_| false,
        ) else {
            return Err(std::io::Error::other("explicit claim accepted unresolved target").into());
        };
        assert_eq!(
            refusal.message(),
            "target `missing` does not resolve to a local branch"
        );
        let automatic = resolve_claim_target(
            directory.path(),
            Some(&claimant),
            TargetSelectionRequest::AutomaticAcquisition,
            &trunk,
            |_| false,
        )
        .map_err(|error| std::io::Error::other(error.message()))?;
        assert_eq!(automatic.target, trunk);
        assert!(automatic.fallback.is_some());
        Ok(())
    }

    #[test]
    fn branch_setting_uses_exact_subsection_and_last_case_insensitive_key() {
        let config = "[branch \"lane-other\"]\n cargoBerthTarget = wrong\n[branch \"lane\"]\n CargoBerthTarget = \"integration\"\n cargoberthtarget = final\n[branch \"else\"]\n cargoBerthTarget = wrong\n";
        assert_eq!(
            branch_target_from_config_text(config, "lane").as_deref(),
            Some("final")
        );
        assert_eq!(branch_target_from_config_text(config, "missing"), None);
    }

    #[test]
    fn target_argument_accepts_only_local_branches() {
        assert_eq!(
            IntegrationTarget::from_branch_argument("integration")
                .map(|target| target.short_name().to_owned()),
            Ok("integration".to_owned())
        );
        assert_eq!(
            IntegrationTarget::from_branch_argument("refs/heads/integration")
                .map(|target| target.short_name().to_owned()),
            Ok("integration".to_owned())
        );
        assert!(
            IntegrationTarget::from_branch_argument("refs/remotes/origin/integration").is_err()
        );
        assert!(
            serde_json::from_str::<IntegrationTarget>("\"refs/remotes/origin/integration\"")
                .is_err()
        );
    }
}
