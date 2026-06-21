use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use dirs::config_dir;
use serde::Deserialize;
use toml::from_str;
use toml_edit::DocumentMut;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::value;

use super::constants::APP_NAME;
use super::constants::GLOBAL_CONFIG_FILE;
use super::diagnostic_code::DiagnosticCode;
use super::diagnostics_config::DiagnosticsConfig;
use super::prelude_pub_mod::PreludePubMod;
use crate::constants::HELP_URL_BASE;

const PRELUDE_KEY: &str = "allow_prelude_pub_mod";
const PRELUDE_COMMENT: &str = "# default-on; set false to review crate-root prelude modules too\n";

/// Resolved global configuration: the diagnostics defaults plus the prelude switch.
#[derive(Debug, Default)]
pub(crate) struct GlobalConfig {
    pub(crate) diagnostics:     DiagnosticsConfig,
    pub(crate) prelude_pub_mod: PreludePubMod,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalConfigFile {
    #[serde(default, rename = "diagnostics")]
    diagnostics_config: DiagnosticsConfig,
    #[serde(default, rename = "visibility")]
    visibility:         GlobalVisibility,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalVisibility {
    #[serde(default, rename = "allow_prelude_pub_mod")]
    prelude_pub_mod: PreludePubMod,
}

impl From<GlobalConfigFile> for GlobalConfig {
    fn from(file: GlobalConfigFile) -> Self {
        Self {
            diagnostics:     file.diagnostics_config,
            prelude_pub_mod: file.visibility.prelude_pub_mod,
        }
    }
}

pub(crate) fn global_config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(APP_NAME).join(GLOBAL_CONFIG_FILE))
}

pub(crate) fn load_global_config() -> GlobalConfig {
    let Some(path) = global_config_path() else {
        return GlobalConfig::default();
    };

    let _ = reconcile_global_config(&path);

    let Ok(contents) = fs::read_to_string(&path) else {
        return GlobalConfig::default();
    };

    from_str::<GlobalConfigFile>(&contents)
        .map_or_else(|_| GlobalConfig::default(), GlobalConfig::from)
}

/// Ensure the global config file exists and lists every known key. Missing keys are
/// inserted with their defaults; existing keys, comments, and ordering are preserved.
/// Writes only when a key was inserted.
fn reconcile_global_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return create_default_global_config(path);
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read global config {}", path.display()))?;
    let mut doc = contents
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse global config {}", path.display()))?;

    let mut inserted = false;

    if let Some(diagnostics) = ensure_table(doc.as_table_mut(), "diagnostics") {
        for code in DiagnosticCode::ALL {
            if !diagnostics.contains_key(code.as_str()) {
                diagnostics.insert(code.as_str(), value(true));
                inserted = true;
            }
        }
    }

    if let Some(visibility) = ensure_table(doc.as_table_mut(), "visibility")
        && !visibility.contains_key(PRELUDE_KEY)
    {
        visibility.insert(PRELUDE_KEY, value(true));
        if let Some(mut key) = visibility.key_mut(PRELUDE_KEY) {
            key.leaf_decor_mut().set_prefix(PRELUDE_COMMENT);
        }
        inserted = true;
    }

    if inserted {
        fs::write(path, doc.to_string())
            .with_context(|| format!("failed to write global config {}", path.display()))?;
    }
    Ok(())
}

/// Returns the named table, inserting an empty one if absent. `None` only when the
/// key already exists as a non-table value (a malformed config we leave untouched).
fn ensure_table<'a>(root: &'a mut Table, name: &str) -> Option<&'a mut Table> {
    root.entry(name)
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(false);
            Item::Table(table)
        })
        .as_table_mut()
}

fn default_global_config_toml() -> String {
    let mut out = format!(
        "# cargo-mend global configuration\n\
         # See {HELP_URL_BASE}#diagnostics for details on each rule.\n\
         # Per-project overrides go in mend.toml at your project or workspace root.\n\
         \n\
         [diagnostics]\n"
    );
    for code in DiagnosticCode::ALL {
        let _ = writeln!(out, "{} = true", code.as_str());
    }
    out.push_str("\n[visibility]\n");
    out.push_str(PRELUDE_COMMENT);
    let _ = writeln!(out, "{PRELUDE_KEY} = true");
    out
}

fn create_default_global_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, default_global_config_toml())
        .with_context(|| format!("failed to write default config to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use toml::from_str;

    use super::GlobalConfig;
    use super::GlobalConfigFile;
    use super::PRELUDE_KEY;
    use super::default_global_config_toml;
    use super::reconcile_global_config;
    use crate::config::DiagnosticCode;
    use crate::config::DiagnosticStatus;
    use crate::config::PreludePubMod;

    #[test]
    fn default_global_config_toml_parses_correctly() {
        let result: Result<GlobalConfigFile, _> = from_str(&default_global_config_toml());
        assert!(result.is_ok(), "default_global_config_toml() should parse");
        let global_config_file = result.unwrap();
        for (code, enabled) in global_config_file.diagnostics_config.entries() {
            assert!(
                matches!(enabled, DiagnosticStatus::Enabled),
                "default config should have {} enabled",
                code.as_str()
            );
        }
        let global = GlobalConfig::from(global_config_file);
        assert_eq!(global.prelude_pub_mod, PreludePubMod::Allowed);
    }

    #[test]
    fn partial_toml_uses_defaults_for_missing_fields() {
        let toml_str = r"
[diagnostics]
prefer_module_import = false
";
        let global_config_file: GlobalConfigFile = from_str(toml_str).unwrap();
        assert!(matches!(
            global_config_file
                .diagnostics_config
                .is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Disabled
        ));
        assert!(matches!(
            global_config_file
                .diagnostics_config
                .is_enabled(DiagnosticCode::ForbiddenPubCrate),
            DiagnosticStatus::Enabled
        ));
        assert_eq!(
            GlobalConfig::from(global_config_file).prelude_pub_mod,
            PreludePubMod::Allowed
        );
    }

    #[test]
    fn reconcile_creates_canonical_default_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        reconcile_global_config(&path).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let file: GlobalConfigFile = from_str(&contents).unwrap();
        for (_, enabled) in file.diagnostics_config.entries() {
            assert!(matches!(enabled, DiagnosticStatus::Enabled));
        }
        assert!(contents.contains(PRELUDE_KEY));
    }

    #[test]
    fn reconcile_preserves_comments_and_explicit_values_when_complete() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, default_global_config_toml()).unwrap();
        // mutate a value and add a user comment, then write a complete file.
        let mut original = std::fs::read_to_string(&path).unwrap();
        original = original.replace(
            "prefer_module_import = true",
            "# my note\nprefer_module_import = false",
        );
        std::fs::write(&path, &original).unwrap();

        reconcile_global_config(&path).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, original, "complete file must be left untouched");
        assert!(after.contains("# my note"));
    }

    #[test]
    fn reconcile_inserts_missing_keys_and_keeps_comments() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        // an old-style file: diagnostics only, no [visibility], with a user comment.
        std::fs::write(
            &path,
            "# user header\n[diagnostics]\nreview_pub_mod = false\n",
        )
        .unwrap();

        reconcile_global_config(&path).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# user header"), "user comment preserved");
        assert!(
            after.contains("review_pub_mod = false"),
            "explicit value preserved"
        );
        assert!(after.contains(PRELUDE_KEY), "prelude key inserted");

        let file: GlobalConfigFile = from_str(&after).unwrap();
        assert!(matches!(
            file.diagnostics_config
                .is_enabled(DiagnosticCode::ReviewPubMod),
            DiagnosticStatus::Disabled
        ));
        for code in DiagnosticCode::ALL {
            assert!(after.contains(code.as_str()), "{} present", code.as_str());
        }

        // second run is a no-op.
        reconcile_global_config(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(second, after, "reconcile is idempotent");
    }
}
