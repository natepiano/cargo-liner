//! Read rule identities supplied by the caller.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::Deserialize;

use crate::constants::UNCOVERED;

#[derive(Deserialize)]
struct RuleDefinition {
    tier:       String,
    family:     RuleFamily,
    surface:    CandidateSurface,
    blob:       String,
    identities: HashMap<String, JudgmentIdentity>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    Names,
    Values,
    Types,
    Modules,
}

impl Family {
    pub(crate) fn from_pin(text: &str) -> RuleFamily {
        for line in text.split('\n') {
            if let Some(family) = line.strip_prefix("family:") {
                let family = match family.trim_matches([' ', '\t']) {
                    "names" => Self::Names,
                    "values" => Self::Values,
                    "types" => Self::Types,
                    "modules" => Self::Modules,
                    _ => continue,
                };
                return RuleFamily::Known(family);
            }
        }
        RuleFamily::Unspecified
    }

    pub(crate) const fn is_line_local(self) -> bool { matches!(self, Self::Names | Self::Values) }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
pub(crate) enum RuleFamily {
    Known(Family),
    Unspecified,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CandidateSurface {
    Classified(Surface),
    Unspecified,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Surface {
    Partial,
    Whole,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JudgmentIdentity {
    Valid(String),
    Unavailable,
}

#[derive(Deserialize)]
pub(crate) struct RuleSnapshot {
    rules: HashMap<String, RuleDefinition>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RuleSnapshotFailure {
    #[error("cannot open the rule snapshot")]
    Open(#[from] std::io::Error),
    #[error("cannot decode the rule snapshot")]
    Decode(#[from] serde_json::Error),
    #[error("rule snapshot lacks blob {blob:?} for rule {rule:?}")]
    MissingIdentity { rule: String, blob: String },
}

pub(crate) enum PinComparison {
    Matches,
    Changed,
    InvalidComponent,
}

impl RuleSnapshot {
    pub(crate) fn read(path: &Path) -> Result<Self, RuleSnapshotFailure> {
        Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
    }

    pub(crate) fn current_family(
        &self,
        rule: &str,
        pin: &str,
    ) -> Result<RuleFamily, RuleSnapshotFailure> {
        let base = rule.strip_suffix(UNCOVERED).unwrap_or(rule);
        let Some(definition) = self.rules.get(base) else {
            return Ok(RuleFamily::Unspecified);
        };
        Ok(if definition.matches(base, pin, &definition.blob)? {
            definition.family
        } else {
            RuleFamily::Unspecified
        })
    }

    pub(crate) fn compares_pins(
        &self,
        rule: &str,
        pin: &str,
        holder: &str,
    ) -> Result<PinComparison, RuleSnapshotFailure> {
        let base = rule.strip_suffix(UNCOVERED).unwrap_or(rule);
        let Some(definition) = self.rules.get(base) else {
            return Ok(PinComparison::InvalidComponent);
        };
        if definition.tier == "unclassified"
            || (rule.ends_with(UNCOVERED)
                && (definition.tier != "B"
                    || !matches!(
                        definition.surface,
                        CandidateSurface::Classified(Surface::Partial)
                    )))
        {
            return Ok(PinComparison::InvalidComponent);
        }
        Ok(
            if definition.matches(
                base,
                holder,
                if pin.is_empty() {
                    &definition.blob
                } else {
                    pin
                },
            )? {
                PinComparison::Matches
            } else {
                PinComparison::Changed
            },
        )
    }
}

impl RuleDefinition {
    fn matches(&self, rule: &str, old: &str, new: &str) -> Result<bool, RuleSnapshotFailure> {
        let previous = self.identity(rule, old)?;
        let current = self.identity(rule, new)?;
        Ok(matches!((previous, current),
            (JudgmentIdentity::Valid(previous), JudgmentIdentity::Valid(current))
            if !previous.is_empty() && previous == current))
    }

    fn identity(&self, rule: &str, blob: &str) -> Result<&JudgmentIdentity, RuleSnapshotFailure> {
        self.identities
            .get(blob)
            .ok_or_else(|| RuleSnapshotFailure::MissingIdentity {
                rule: rule.to_owned(),
                blob: blob.to_owned(),
            })
    }
}
