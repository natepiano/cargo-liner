//! Filesystem loader for user theme directories.
//!
//! Scans a directory for `*.toml` files, parses each as a
//! [`ThemeFamily`], and registers every contained variant on a
//! [`ThemeRegistry`]. Each registered variant inherits the app roles
//! its file leaves out from a built-in. Files that fail to read or
//! parse are recorded via [`ThemeRegistry::record_failed_file`] so the
//! UI can surface the error.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use super::Appearance;
use super::StyleSpec;
use super::ThemeFamily;
use super::ThemeId;
use super::ThemeLoadError;
use super::ThemeRegistry;
use super::ThemeVariant;

struct FailedFile {
    path:  PathBuf,
    error: ThemeLoadError,
}

impl ThemeRegistry {
    /// Build a registry seeded with the app's `builtins` and extended
    /// with every variant registered from `dir` (if `Some`).
    ///
    /// `builtins` is the app's own compiled-in set — see
    /// [`ThemeRegistry::new_with_builtins`]. A user variant sharing an
    /// id with one of them replaces it in place.
    ///
    /// A user variant inherits every role in [`Theme::roles`] its file
    /// leaves out, so a theme file written before the app added a role
    /// still styles it. The roles come from the built-in the variant
    /// replaces, or, for a new id, from the first built-in of its
    /// appearance: the variant [`ThemeRegistry::resolve_active`] falls
    /// back to on a miss. Roles the file sets win. The built-ins' roles
    /// are read before any user variant registers, so inheritance never
    /// draws on another user variant.
    ///
    /// Returns the assembled registry; the caller installs or replaces
    /// it via [`crate::install_theme_state`] / [`crate::replace_registry`].
    ///
    /// [`Theme::roles`]: super::Theme::roles
    #[must_use]
    pub fn from_dir_with_builtins(dir: Option<&Path>, builtins: Vec<ThemeVariant>) -> Self {
        let builtin_roles: Vec<BuiltinRoles> = builtins.iter().map(BuiltinRoles::of).collect();
        let mut registry = Self::new_with_builtins(builtins);
        let Some(dir) = dir else {
            return registry;
        };
        let (loaded, failed) = scan_themes_dir(dir);
        for family in loaded {
            for variant_file in family.variants {
                let id = ThemeId::new(variant_file.name.clone());
                let appearance = variant_file.appearance;
                let mut theme = variant_file.into_theme();
                inherit_roles(&builtin_roles, &id, appearance, &mut theme.roles);
                registry.register(ThemeVariant {
                    id,
                    appearance,
                    theme,
                });
            }
        }
        for FailedFile { path, error } in failed {
            registry.record_failed_file(path, error);
        }
        registry
    }
}

/// One built-in's roles, read before registration so a user variant
/// that replaces the built-in in place cannot change them.
struct BuiltinRoles {
    id:         ThemeId,
    appearance: Appearance,
    roles:      BTreeMap<String, StyleSpec>,
}

impl BuiltinRoles {
    /// The id, appearance and roles of `variant`.
    fn of(variant: &ThemeVariant) -> Self {
        Self {
            id:         variant.id.clone(),
            appearance: variant.appearance,
            roles:      variant.theme.roles.clone(),
        }
    }
}

/// Add to `roles` every role it leaves out from the built-in sharing
/// `id`, or, when `id` names no built-in, from the first built-in of
/// `appearance`. With neither, `roles` stays as it is.
fn inherit_roles(
    builtins: &[BuiltinRoles],
    id: &ThemeId,
    appearance: Appearance,
    roles: &mut BTreeMap<String, StyleSpec>,
) {
    let source = builtins
        .iter()
        .find(|builtin| &builtin.id == id)
        .or_else(|| {
            builtins
                .iter()
                .find(|builtin| builtin.appearance == appearance)
        });
    let Some(source) = source else {
        return;
    };
    for (role, spec) in &source.roles {
        roles.entry(role.clone()).or_insert(*spec);
    }
}

/// Read every `*.toml` under `dir` in sorted ASCII filename order.
///
/// Sorted iteration is what makes the "later file overrides earlier"
/// tie-break deterministic across runs. Returns the parsed families
/// alongside any files that failed to parse; neither is fatal, and the
/// caller decides whether to toast or just log.
fn scan_themes_dir(dir: &Path) -> (Vec<ThemeFamily>, Vec<FailedFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
        })
        .collect();
    paths.sort();

    let mut loaded = Vec::with_capacity(paths.len());
    let mut failed = Vec::new();
    for path in paths {
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<ThemeFamily>(&contents) {
                Ok(family) => loaded.push(family),
                Err(err) => failed.push(FailedFile {
                    path,
                    error: ThemeLoadError::new(format!("parse error: {err}")),
                }),
            },
            Err(err) => failed.push(FailedFile {
                path,
                error: ThemeLoadError::new(format!("read error: {err}")),
            }),
        }
    }
    (loaded, failed)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;

    use ratatui::style::Color;
    use tempfile::TempDir;

    use super::*;
    use crate::theme;

    const SAMPLE_VARIANT_NAME: &str = "Sample Dark";
    const SECOND_DARK_NAME: &str = "Second Dark";
    const CUSTOM_DARK_NAME: &str = "Custom Dark";
    const SHARED_ROLE: &str = "shared";
    const FIRST_DARK_ROLE: &str = "first_dark";
    const SECOND_DARK_ROLE: &str = "second_dark";
    const OWN_ROLE: &str = "own";

    fn write_file(path: &Path, contents: &str) {
        let mut f = fs::File::create(path).expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write temp file");
        f.sync_all().expect("sync temp file");
    }

    const SAMPLE_FAMILY: &str = include_str!("testdata/sample_family.toml");

    /// Two stand-ins for an app's compiled-in set. The dark one shares
    /// its id with the variant in `SAMPLE_FAMILY` so the override path
    /// has something to collide with.
    fn app_builtins() -> Vec<ThemeVariant> {
        vec![
            ThemeVariant {
                id:         ThemeId::new(SAMPLE_VARIANT_NAME),
                appearance: Appearance::Dark,
                theme:      theme::fallback_theme(Appearance::Dark),
            },
            ThemeVariant {
                id:         ThemeId::new("Sample Light"),
                appearance: Appearance::Light,
                theme:      theme::fallback_theme(Appearance::Light),
            },
        ]
    }

    /// `app_builtins` plus a second dark variant, each carrying roles.
    /// The two dark ones each hold a role the other lacks, so an
    /// inherited role shows which built-in it came from.
    fn builtins_with_roles() -> Vec<ThemeVariant> {
        let mut builtins = app_builtins();
        builtins.push(ThemeVariant {
            id:         ThemeId::new(SECOND_DARK_NAME),
            appearance: Appearance::Dark,
            theme:      theme::fallback_theme(Appearance::Dark),
        });
        builtins[0].theme.roles =
            roles_of(&[(SHARED_ROLE, Color::Red), (FIRST_DARK_ROLE, Color::Green)]);
        builtins[1].theme.roles = roles_of(&[(SHARED_ROLE, Color::Blue)]);
        builtins[2].theme.roles = roles_of(&[
            (SHARED_ROLE, Color::Yellow),
            (SECOND_DARK_ROLE, Color::Magenta),
        ]);
        builtins
    }

    fn roles_of(roles: &[(&str, Color)]) -> BTreeMap<String, StyleSpec> {
        roles
            .iter()
            .map(|(role, color)| ((*role).to_string(), StyleSpec::from_color(*color)))
            .collect()
    }

    /// `SAMPLE_FAMILY` with its variant renamed to `name` and `roles`,
    /// one `role = "Color"` line each, written into its roles table.
    fn family_with_roles(name: &str, roles: &str) -> String {
        SAMPLE_FAMILY.replace(SAMPLE_VARIANT_NAME, name).replace(
            "[variants.roles]\n",
            &format!("[variants.roles]\n{roles}\n"),
        )
    }

    fn roles_in(registry: &ThemeRegistry, name: &str) -> BTreeMap<String, StyleSpec> {
        registry
            .find(&ThemeId::new(name))
            .expect("registered variant")
            .theme
            .roles
            .clone()
    }

    #[test]
    fn missing_directory_returns_only_builtins() {
        let registry = ThemeRegistry::from_dir_with_builtins(
            Some(Path::new("/definitely/not/a/dir/xyzzy")),
            app_builtins(),
        );
        assert_eq!(registry.len(), 2);
        assert!(registry.find(&ThemeId::new(SAMPLE_VARIANT_NAME)).is_some());
        assert!(registry.find(&ThemeId::new("Sample Light")).is_some());
        assert_eq!(
            registry.status().failed_files,
            [] as [(PathBuf, ThemeLoadError); 0]
        );
    }

    #[test]
    fn no_directory_argument_returns_only_builtins() {
        let registry = ThemeRegistry::from_dir_with_builtins(None, app_builtins());
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn user_variant_overrides_builtin_with_same_name() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(&dir.path().join("override.toml"), SAMPLE_FAMILY);
        let registry = ThemeRegistry::from_dir_with_builtins(Some(dir.path()), app_builtins());
        assert_eq!(registry.len(), 2, "override must replace in place");
        assert_eq!(
            registry.status().overridden,
            vec![ThemeId::new(SAMPLE_VARIANT_NAME)]
        );
    }

    #[test]
    fn parse_failure_is_recorded_not_fatal() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(&dir.path().join("bad.toml"), "this is not = valid toml [\n");
        let registry = ThemeRegistry::from_dir_with_builtins(Some(dir.path()), app_builtins());
        assert_eq!(registry.len(), 2, "built-ins survive a parse error");
        assert_eq!(registry.status().failed_files.len(), 1);
        let (path, err) = &registry.status().failed_files[0];
        assert!(path.ends_with("bad.toml"));
        assert!(err.message().contains("parse error"));
    }

    #[test]
    fn a_replacing_variant_inherits_the_roles_it_leaves_out_from_the_builtin_it_replaces() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(
            &dir.path().join("second.toml"),
            &family_with_roles(SECOND_DARK_NAME, &format!("{SHARED_ROLE} = \"White\"")),
        );
        let registry =
            ThemeRegistry::from_dir_with_builtins(Some(dir.path()), builtins_with_roles());
        assert_eq!(
            roles_in(&registry, SECOND_DARK_NAME),
            roles_of(&[
                (SHARED_ROLE, Color::White),
                (SECOND_DARK_ROLE, Color::Magenta)
            ])
        );
    }

    #[test]
    fn a_new_variant_inherits_from_the_first_builtin_of_its_appearance() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(
            &dir.path().join("custom.toml"),
            &family_with_roles(CUSTOM_DARK_NAME, &format!("{OWN_ROLE} = \"Cyan\"")),
        );
        let registry =
            ThemeRegistry::from_dir_with_builtins(Some(dir.path()), builtins_with_roles());
        assert_eq!(
            roles_in(&registry, CUSTOM_DARK_NAME),
            roles_of(&[
                (OWN_ROLE, Color::Cyan),
                (SHARED_ROLE, Color::Red),
                (FIRST_DARK_ROLE, Color::Green),
            ])
        );
    }

    /// `a.toml` replaces the first dark built-in before `b.toml` adds a
    /// new dark variant; the new one still inherits the compiled-in
    /// roles.
    #[test]
    fn inherited_roles_come_from_the_builtins_as_compiled_not_a_user_replacement() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(
            &dir.path().join("a.toml"),
            &family_with_roles(
                SAMPLE_VARIANT_NAME,
                &format!("{SHARED_ROLE} = \"White\"\n{FIRST_DARK_ROLE} = \"White\""),
            ),
        );
        write_file(
            &dir.path().join("b.toml"),
            &family_with_roles(CUSTOM_DARK_NAME, ""),
        );
        let registry =
            ThemeRegistry::from_dir_with_builtins(Some(dir.path()), builtins_with_roles());
        assert_eq!(
            roles_in(&registry, SAMPLE_VARIANT_NAME),
            roles_of(&[(SHARED_ROLE, Color::White), (FIRST_DARK_ROLE, Color::White)])
        );
        assert_eq!(
            roles_in(&registry, CUSTOM_DARK_NAME),
            roles_of(&[(SHARED_ROLE, Color::Red), (FIRST_DARK_ROLE, Color::Green)])
        );
    }

    /// cargo-port's built-ins carry no roles, so its theme files load
    /// with exactly the roles they set.
    #[test]
    fn builtins_without_roles_leave_a_files_roles_as_written() {
        let dir = TempDir::new().expect("create temp themes dir");
        write_file(
            &dir.path().join("override.toml"),
            &family_with_roles(SAMPLE_VARIANT_NAME, &format!("{OWN_ROLE} = \"Cyan\"")),
        );
        let registry = ThemeRegistry::from_dir_with_builtins(Some(dir.path()), app_builtins());
        assert_eq!(
            roles_in(&registry, SAMPLE_VARIANT_NAME),
            roles_of(&[(OWN_ROLE, Color::Cyan)])
        );
    }
}
