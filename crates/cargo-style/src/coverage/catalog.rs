//! Read the coverage policy and pin identities from the same catalog.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::CoverageError;
use super::lint::LintTool;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::RuleFamily;
use crate::rule_snapshot::RuleSnapshot;
use crate::site_ledger::InputScope;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Tier {
    A,
    B,
    C,
    D,
    E,
    #[serde(rename = "unclassified")]
    Unclassified,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RuleInputScope {
    File,
    Crate,
    Manifest,
}

impl From<RuleInputScope> for InputScope {
    fn from(scope: RuleInputScope) -> Self {
        match scope {
            RuleInputScope::File => Self::File,
            RuleInputScope::Crate => Self::Crate,
            RuleInputScope::Manifest => Self::Manifest,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum RuleReach {
    Member(String),
    Repository,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
pub(super) enum Mechanism {
    Lint(LintTool),
    Manual,
}

#[derive(Deserialize)]
pub(super) struct CoverageRule {
    pub(super) tier:        Tier,
    pub(super) family:      RuleFamily,
    pub(super) input_scope: RuleInputScope,
    pub(super) reach:       RuleReach,
    pub(super) mechanism:   Mechanism,
}

impl CoverageRule {
    pub(super) const fn family_name(&self) -> Option<&'static str> {
        match self.family {
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
