use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::StoredReport;
use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
use crate::compiler::settings;

/// Whether this unit's persisted analysis can answer the current request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ReportCacheState {
    Compatible,
    AnotherMendBuild,
    InputsChanged,
    Missing,
    Unreadable,
}

impl ReportCacheState {
    pub(in crate::compiler) fn read(path: &Path, config_fingerprint: &str) -> Self {
        let text = match fs::read(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Self::Missing,
            Err(_) => return Self::Unreadable,
        };
        serde_json::from_slice::<StoredReport>(&text).map_or(Self::Unreadable, |stored| {
            Self::for_report(&stored, config_fingerprint)
        })
    }

    pub(in crate::compiler) fn for_report(stored: &StoredReport, config_fingerprint: &str) -> Self {
        if stored.analysis_fingerprint != settings::current_analysis_fingerprint() {
            return Self::AnotherMendBuild;
        }
        if stored.version != FINDINGS_SCHEMA_VERSION
            || stored.config_fingerprint != config_fingerprint
            || !stored_crate_root_exists(stored)
        {
            return Self::InputsChanged;
        }
        Self::Compatible
    }
}

fn stored_crate_root_exists(stored: &StoredReport) -> bool {
    stored.crate_root_file.is_empty() || {
        let crate_root = Path::new(&stored.crate_root_file);
        if crate_root.is_absolute() {
            crate_root.exists()
        } else {
            Path::new(&stored.package_root).join(crate_root).exists()
        }
    }
}
