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
use toml_edit::Key;
use toml_edit::Table;
use toml_edit::value;

use super::constants::APP_NAME;
use super::constants::CONFIG_VERSION;
use super::constants::CONFIG_VERSION_KEY;
use super::constants::DIAGNOSTICS_TABLE_KEY;
use super::constants::GLOBAL_CONFIG_FILE;
use super::constants::LEGACY_OVERBROAD_PUB_CRATE_KEY;
use super::constants::PRELUDE_COMMENT;
use super::constants::PRELUDE_KEY;
use super::constants::PUB_IN_PATH_COMMENT;
use super::constants::PUB_IN_PATH_KEY;
use super::constants::VISIBILITY_TABLE_KEY;
use super::diagnostic_code::DiagnosticCode;
use super::diagnostics_config::DiagnosticsConfig;
use super::prelude_pub_mod::PreludePubMod;
use super::pub_in_path::PubInPath;
use crate::constants::HELP_URL_BASE;

/// Resolved global configuration: diagnostics defaults and visibility settings.
#[derive(Debug, Default)]
pub(crate) struct GlobalConfig {
    pub(crate) diagnostics:     DiagnosticsConfig,
    pub(super) prelude_pub_mod: PreludePubMod,
    pub(super) pub_in_path:     PubInPath,
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
    #[serde(default)]
    pub_in_path:     PubInPath,
}

impl From<GlobalConfigFile> for GlobalConfig {
    fn from(file: GlobalConfigFile) -> Self {
        Self {
            diagnostics:     file.diagnostics_config,
            prelude_pub_mod: file.visibility.prelude_pub_mod,
            pub_in_path:     file.visibility.pub_in_path,
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

/// Ensure the global config file exists, lists every known key, and carries the
/// current `config_version`. Missing keys are inserted with their defaults;
/// existing keys, comments, and ordering are preserved. Writes only when
/// something changed.
fn reconcile_global_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return create_default_global_config(path);
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read global config {}", path.display()))?;
    let mut doc = contents
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse global config {}", path.display()))?;

    let mut edit = ConfigEdit::Unchanged;

    if let Some(diagnostics) = ensure_table(doc.as_table_mut(), DIAGNOSTICS_TABLE_KEY) {
        if matches!(
            migrate_overbroad_pub_crate_key(diagnostics),
            ConfigEdit::Rewritten
        ) {
            edit = ConfigEdit::Rewritten;
        }
        for code in DiagnosticCode::ALL {
            if !diagnostics.contains_key(code.as_str()) {
                diagnostics.insert(code.as_str(), value(true));
                edit = ConfigEdit::Rewritten;
            }
        }
    }

    if let Some(visibility) = ensure_table(doc.as_table_mut(), VISIBILITY_TABLE_KEY)
        && !visibility.contains_key(PRELUDE_KEY)
    {
        visibility.insert(PRELUDE_KEY, value(true));
        if let Some(mut key) = visibility.key_mut(PRELUDE_KEY) {
            key.leaf_decor_mut().set_prefix(PRELUDE_COMMENT);
        }
        edit = ConfigEdit::Rewritten;
    }

    if let Some(visibility) = ensure_table(doc.as_table_mut(), VISIBILITY_TABLE_KEY)
        && !visibility.contains_key(PUB_IN_PATH_KEY)
    {
        visibility.insert(PUB_IN_PATH_KEY, value(PubInPath::Required.config_value()));
        if let Some(mut key) = visibility.key_mut(PUB_IN_PATH_KEY) {
            key.leaf_decor_mut().set_prefix(PUB_IN_PATH_COMMENT);
        }
        edit = ConfigEdit::Rewritten;
    }

    if matches!(migrate_unversioned_config(&mut doc), ConfigEdit::Rewritten) {
        edit = ConfigEdit::Rewritten;
    }

    if matches!(edit, ConfigEdit::Rewritten) {
        fs::write(path, doc.to_string())
            .with_context(|| format!("failed to write global config {}", path.display()))?;
    }
    Ok(())
}

/// Whether reconciliation touched the parsed document and the file must be
/// written back.
#[derive(Clone, Copy)]
enum ConfigEdit {
    Unchanged,
    Rewritten,
}

fn migrate_overbroad_pub_crate_key(diagnostics: &mut Table) -> ConfigEdit {
    let current_key = DiagnosticCode::OverbroadPubCrate.as_str();
    if diagnostics.contains_key(current_key) {
        return diagnostics
            .remove(LEGACY_OVERBROAD_PUB_CRATE_KEY)
            .map_or(ConfigEdit::Unchanged, |_| ConfigEdit::Rewritten);
    }

    let Some((legacy_key, item)) = diagnostics.remove_entry(LEGACY_OVERBROAD_PUB_CRATE_KEY) else {
        return ConfigEdit::Unchanged;
    };
    let current_key = Key::new(current_key)
        .with_leaf_decor(legacy_key.leaf_decor().clone())
        .with_dotted_decor(legacy_key.dotted_decor().clone());
    diagnostics.insert_formatted(&current_key, item);
    ConfigEdit::Rewritten
}

/// A config with no `config_version` predates the `pub_in_path` default flip, so
/// the `permitted` this tool inserted on its behalf is not a user choice —
/// rewrite it to the new default and refresh its comment. Any other value is a
/// deliberate setting and is kept. Stamping the version is what keeps this a
/// one-time migration: an explicit `permitted` written afterwards survives every
/// later run.
fn migrate_unversioned_config(doc: &mut DocumentMut) -> ConfigEdit {
    if doc.contains_key(CONFIG_VERSION_KEY) {
        return ConfigEdit::Unchanged;
    }

    if let Some(visibility) = ensure_table(doc.as_table_mut(), VISIBILITY_TABLE_KEY)
        && visibility.get(PUB_IN_PATH_KEY).and_then(Item::as_str)
            == Some(PubInPath::Permitted.config_value())
    {
        visibility.insert(PUB_IN_PATH_KEY, value(PubInPath::Required.config_value()));
        if let Some(mut key) = visibility.key_mut(PUB_IN_PATH_KEY) {
            key.leaf_decor_mut().set_prefix(PUB_IN_PATH_COMMENT);
        }
    }

    doc.insert(CONFIG_VERSION_KEY, value(CONFIG_VERSION));
    ConfigEdit::Rewritten
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
         {CONFIG_VERSION_KEY} = {CONFIG_VERSION}\n\
         \n\
         [diagnostics]\n"
    );
    for code in DiagnosticCode::ALL {
        let _ = writeln!(out, "{} = true", code.as_str());
    }
    out.push_str("\n[visibility]\n");
    out.push_str(PRELUDE_COMMENT);
    let _ = writeln!(out, "{PRELUDE_KEY} = true");
    out.push_str(PUB_IN_PATH_COMMENT);
    let _ = writeln!(
        out,
        "{PUB_IN_PATH_KEY} = \"{}\"",
        PubInPath::Required.config_value()
    );
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

    use super::CONFIG_VERSION;
    use super::CONFIG_VERSION_KEY;
    use super::GlobalConfig;
    use super::GlobalConfigFile;
    use super::LEGACY_OVERBROAD_PUB_CRATE_KEY;
    use super::PRELUDE_KEY;
    use super::PUB_IN_PATH_COMMENT;
    use super::PUB_IN_PATH_KEY;
    use super::PubInPath;
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
        assert_eq!(global.pub_in_path, PubInPath::Required);
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
                .is_enabled(DiagnosticCode::OverbroadPubCrate),
            DiagnosticStatus::Enabled
        ));
        let global = GlobalConfig::from(global_config_file);
        assert_eq!(global.prelude_pub_mod, PreludePubMod::Allowed);
        assert_eq!(global.pub_in_path, PubInPath::Required);
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
        assert!(contents.contains(PUB_IN_PATH_KEY));
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
    fn reconcile_migrates_legacy_overbroad_pub_crate_key() {
        let current_key = DiagnosticCode::OverbroadPubCrate.as_str();
        let original = format!(
            "{CONFIG_VERSION_KEY} = {CONFIG_VERSION}\n[diagnostics]\n# keep this setting\n{LEGACY_OVERBROAD_PUB_CRATE_KEY} = false\n"
        );

        let after = reconciled(&original);

        assert!(!after.contains(LEGACY_OVERBROAD_PUB_CRATE_KEY));
        assert!(after.contains(&format!("# keep this setting\n{current_key} = false")));
        let file: GlobalConfigFile = from_str(&after).unwrap();
        assert_eq!(
            file.diagnostics_config
                .is_enabled(DiagnosticCode::OverbroadPubCrate),
            DiagnosticStatus::Disabled
        );

        let twice = reconciled(&after);
        assert_eq!(twice, after, "diagnostic-key migration must be idempotent");
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
        assert!(after.contains(PUB_IN_PATH_KEY), "pub_in_path key inserted");

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

    #[test]
    fn reconcile_inserts_pub_in_path_without_disturbing_existing_visibility_content() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let pub_in_path_line = format!(
            "{PUB_IN_PATH_COMMENT}{PUB_IN_PATH_KEY} = \"{}\"\n",
            PubInPath::Required.config_value()
        );
        let existing = default_global_config_toml().replace(&pub_in_path_line, "");
        std::fs::write(&path, &existing).unwrap();

        reconcile_global_config(&path).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, format!("{existing}{pub_in_path_line}"));
    }

    /// Writes `contents`, reconciles it once, and returns the file as it stands
    /// afterwards.
    fn reconciled(contents: &str) -> String {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        reconcile_global_config(&path).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    fn visibility_table(pub_in_path: PubInPath) -> String {
        format!(
            "[visibility]\n{PUB_IN_PATH_KEY} = \"{}\"\n",
            pub_in_path.config_value()
        )
    }

    fn resolved_pub_in_path(contents: &str) -> PubInPath {
        GlobalConfig::from(from_str::<GlobalConfigFile>(contents).unwrap()).pub_in_path
    }

    #[test]
    fn unversioned_config_migrates_permitted_to_required() {
        let after = reconciled(&visibility_table(PubInPath::Permitted));

        assert_eq!(resolved_pub_in_path(&after), PubInPath::Required);
        assert!(after.contains(CONFIG_VERSION_KEY), "version stamp added");
        assert!(
            after.contains(PUB_IN_PATH_COMMENT.trim_end()),
            "comment refreshed alongside the value"
        );
    }

    #[test]
    fn unversioned_config_keeps_an_explicitly_chosen_value() {
        for pub_in_path in [PubInPath::Forbidden, PubInPath::Required] {
            let after = reconciled(&visibility_table(pub_in_path));

            assert_eq!(resolved_pub_in_path(&after), pub_in_path);
            assert!(after.contains(CONFIG_VERSION_KEY), "version stamp added");
        }
    }

    #[test]
    fn versioned_config_keeps_permitted_forever() {
        let versioned = format!(
            "{CONFIG_VERSION_KEY} = {CONFIG_VERSION}\n{}",
            visibility_table(PubInPath::Permitted)
        );

        let after = reconciled(&versioned);
        assert_eq!(resolved_pub_in_path(&after), PubInPath::Permitted);

        let twice = reconciled(&after);
        assert_eq!(resolved_pub_in_path(&twice), PubInPath::Permitted);
    }

    #[test]
    fn unversioned_config_without_visibility_table_lands_on_the_default() {
        let after = reconciled("[diagnostics]\nreview_pub_mod = false\n");

        assert_eq!(resolved_pub_in_path(&after), PubInPath::Required);
        assert!(after.contains(CONFIG_VERSION_KEY), "version stamp added");
        assert!(after.contains(PRELUDE_KEY), "prelude key inserted");
    }
}
