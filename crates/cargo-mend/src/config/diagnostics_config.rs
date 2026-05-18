use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use super::diagnostic_code::DiagnosticCode;
use super::diagnostic_status::DiagnosticStatus;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DiagnosticsConfig {
    #[serde(flatten)]
    rules: BTreeMap<DiagnosticCode, bool>,
}

impl DiagnosticsConfig {
    pub(crate) fn is_enabled(&self, code: DiagnosticCode) -> DiagnosticStatus {
        self.rules
            .get(&code)
            .copied()
            .map_or(DiagnosticStatus::Enabled, DiagnosticStatus::from)
    }

    pub(crate) fn entries(&self) -> Vec<(DiagnosticCode, DiagnosticStatus)> {
        DiagnosticCode::ALL
            .iter()
            .map(|code| (*code, self.is_enabled(*code)))
            .collect()
    }

    pub(crate) fn merge_project(&self, project: &Self) -> Self {
        let mut rules = self.rules.clone();
        for (code, enabled) in &project.rules {
            rules.insert(*code, *enabled);
        }
        Self { rules }
    }
}

#[cfg(test)]
mod tests {
    use super::DiagnosticCode;
    use super::DiagnosticStatus;
    use super::DiagnosticsConfig;

    #[test]
    fn is_enabled_reflects_config_values() {
        let mut diagnostics_config = DiagnosticsConfig::default();
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Enabled
        );
        diagnostics_config
            .rules
            .insert(DiagnosticCode::PreferModuleImport, false);
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Disabled
        );
    }

    #[test]
    fn missing_code_defaults_to_enabled() {
        let diagnostics_config = DiagnosticsConfig::default();
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::ForbiddenPubCrate),
            DiagnosticStatus::Enabled
        );
    }

    #[test]
    fn merge_project_overrides_global() {
        let mut global = DiagnosticsConfig::default();
        global
            .rules
            .insert(DiagnosticCode::PreferModuleImport, false);

        let mut project = DiagnosticsConfig::default();
        project
            .rules
            .insert(DiagnosticCode::PreferModuleImport, true);
        project.rules.insert(DiagnosticCode::SuspiciousPub, false);

        let merged = global.merge_project(&project);
        assert_eq!(
            merged.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Enabled
        );
        assert_eq!(
            merged.is_enabled(DiagnosticCode::SuspiciousPub),
            DiagnosticStatus::Disabled
        );
    }
}
