//! Read the coverage policy and pin identities from the same catalog.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::CoverageError;
use super::lint::LintTool;
use crate::rule_snapshot::CandidateSurface;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::RuleFamily;
use crate::rule_snapshot::RuleSnapshot;
use crate::rule_snapshot::Surface;
use crate::sites::Reach;
use crate::sites::Rule;
use crate::sites::RulePolicy;
use crate::sites::Tier;

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
pub(super) enum Mechanism {
    Lint(LintTool),
    Manual,
}

#[derive(Deserialize)]
pub(super) struct CoverageRule {
    blob:                 String,
    reach:                Reach,
    #[serde(flatten)]
    pub(super) policy:    RulePolicy,
    pub(super) mechanism: Mechanism,
}

impl CoverageRule {
    pub(super) fn check_native(&self, id: &str, rule: &Rule) -> Result<(), CoverageError> {
        let same_pin = self.blob == rule.pin;
        if same_pin && self.reach == rule.reach && self.policy == rule.policy {
            Ok(())
        } else {
            Err(CoverageError::InconsistentRule(id.to_owned()))
        }
    }

    pub(super) fn has_uncovered(&self) -> bool {
        self.policy.tier == Tier::B
            && self.policy.surface == CandidateSurface::Declared(Surface::Partial)
    }

    pub(super) const fn family_name(&self) -> Option<&'static str> {
        match self.policy.family {
            RuleFamily::Known(Family::Modules) => Some("modules"),
            RuleFamily::Known(Family::Names) => Some("names"),
            RuleFamily::Known(Family::Types) => Some("types"),
            RuleFamily::Known(Family::Values) => Some("values"),
            RuleFamily::Unspecified => None,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct CoverageCatalog {
    pub(super) rules: BTreeMap<String, CoverageRule>,
}

impl CoverageCatalog {
    pub(super) fn read(path: &Path) -> Result<(Self, RuleSnapshot), CoverageError> {
        let file = File::open(path).map_err(CoverageError::OpenRules)?;
        let catalog: Value =
            serde_json::from_reader(BufReader::new(file)).map_err(CoverageError::DecodeRules)?;
        let rules = serde_json::from_value(catalog.clone()).map_err(CoverageError::DecodeRules)?;
        let pins = serde_json::from_value(catalog).map_err(CoverageError::DecodeRules)?;
        Ok((rules, pins))
    }
}
