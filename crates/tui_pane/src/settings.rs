//! `SettingsRegistry<Ctx>`: declarative settings table the binary
//! authors and (Phase 11+) the `SettingsPane` renders.
//!
//! One impl per app — the binary builds a `SettingsRegistry<Ctx>` and
//! hands it to [`KeymapBuilder::with_settings`](crate::KeymapBuilder).
//! Phase 10 stores it on the keymap; Phase 11+ wires the registry into
//! the settings overlay's row builder.

use crate::AppContext;

/// One declared setting kept on a [`SettingsRegistry`].
///
/// The variant carries the get / set `fn` pointers the registry needs
/// to read and write the underlying field on `Ctx`. Each setting's
/// `name` lives on the outer [`SettingEntry`].
pub enum SettingKind<Ctx: AppContext> {
    /// A `bool`-typed setting.
    Bool {
        /// Read the current value.
        get: fn(&Ctx) -> bool,
        /// Write a new value.
        set: fn(&mut Ctx, bool),
    },
    /// A closed-set enum-typed setting. `variants` is the canonical
    /// label set the UI cycles through; `get` and `set` operate on
    /// those labels.
    Enum {
        /// Read the current label.
        get:      fn(&Ctx) -> &'static str,
        /// Write a new label. Implementations should clamp to a member
        /// of `variants`.
        set:      fn(&mut Ctx, &'static str),
        /// The closed set of valid labels.
        variants: &'static [&'static str],
    },
    /// An integer-typed setting. `bounds`, if present, is `(min, max)`
    /// inclusive; the UI clamps to this range.
    Int {
        /// Read the current value.
        get:    fn(&Ctx) -> i64,
        /// Write a new value.
        set:    fn(&mut Ctx, i64),
        /// Inclusive `(min, max)` bounds, or `None` for unbounded.
        bounds: Option<(i64, i64)>,
    },
}

/// One entry in a [`SettingsRegistry`].
pub struct SettingEntry<Ctx: AppContext> {
    /// Stable, user-visible name (also the key the binary's TOML uses).
    pub name: &'static str,
    /// Type and accessors for this setting.
    pub kind: SettingKind<Ctx>,
}

/// Declarative settings registry, one per app.
///
/// Built with the chained `add_*` methods plus [`Self::with_bounds`].
/// The registry is consumed by
/// [`KeymapBuilder::with_settings`](crate::KeymapBuilder) and surfaced
/// to the settings overlay in Phase 11+.
pub struct SettingsRegistry<Ctx: AppContext> {
    entries: Vec<SettingEntry<Ctx>>,
}

impl<Ctx: AppContext> SettingsRegistry<Ctx> {
    /// Empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a boolean setting.
    #[must_use]
    pub fn add_bool(
        mut self,
        name: &'static str,
        get: fn(&Ctx) -> bool,
        set: fn(&mut Ctx, bool),
    ) -> Self {
        self.entries.push(SettingEntry {
            name,
            kind: SettingKind::Bool { get, set },
        });
        self
    }

    /// Add a closed-set enum setting. `variants` is the label list the
    /// UI cycles through.
    #[must_use]
    pub fn add_enum(
        mut self,
        name: &'static str,
        get: fn(&Ctx) -> &'static str,
        set: fn(&mut Ctx, &'static str),
        variants: &'static [&'static str],
    ) -> Self {
        self.entries.push(SettingEntry {
            name,
            kind: SettingKind::Enum { get, set, variants },
        });
        self
    }

    /// Add an integer setting. Pair with [`Self::with_bounds`] to clamp
    /// the UI to a range.
    #[must_use]
    pub fn add_int(
        mut self,
        name: &'static str,
        get: fn(&Ctx) -> i64,
        set: fn(&mut Ctx, i64),
    ) -> Self {
        self.entries.push(SettingEntry {
            name,
            kind: SettingKind::Int {
                get,
                set,
                bounds: None,
            },
        });
        self
    }

    /// Set inclusive `(min, max)` bounds on the most recently added
    /// integer setting. No-op if the previous entry was not an
    /// [`SettingKind::Int`].
    #[must_use]
    pub fn with_bounds(mut self, min: i64, max: i64) -> Self {
        if let Some(SettingEntry {
            kind: SettingKind::Int { bounds, .. },
            ..
        }) = self.entries.last_mut()
        {
            *bounds = Some((min, max));
        }
        self
    }

    /// Borrow all entries in declaration order.
    #[must_use]
    pub fn entries(&self) -> &[SettingEntry<Ctx>] { &self.entries }
}

impl<Ctx: AppContext> Default for SettingsRegistry<Ctx> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::SettingKind;
    use super::SettingsRegistry;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    struct TestApp {
        framework: Framework<Self>,
        a_bool:    bool,
        a_label:   &'static str,
        a_number:  i64,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
            a_bool:    false,
            a_label:   "off",
            a_number:  3,
        }
    }

    fn get_bool(ctx: &TestApp) -> bool { ctx.a_bool }
    fn set_bool(ctx: &mut TestApp, v: bool) { ctx.a_bool = v; }
    fn get_label(ctx: &TestApp) -> &'static str { ctx.a_label }
    fn set_label(ctx: &mut TestApp, v: &'static str) { ctx.a_label = v; }
    fn get_int(ctx: &TestApp) -> i64 { ctx.a_number }
    fn set_int(ctx: &mut TestApp, v: i64) { ctx.a_number = v; }

    #[test]
    fn empty_registry_has_no_entries() {
        let reg: SettingsRegistry<TestApp> = SettingsRegistry::new();
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn default_matches_new() {
        let reg: SettingsRegistry<TestApp> = SettingsRegistry::default();
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn add_bool_records_entry_and_round_trips() {
        let reg = SettingsRegistry::<TestApp>::new().add_bool("vim", get_bool, set_bool);
        assert_eq!(reg.entries().len(), 1);
        let entry = &reg.entries()[0];
        assert_eq!(entry.name, "vim");
        let SettingKind::Bool { get, set } = entry.kind else {
            panic!("expected Bool variant");
        };
        let mut app = fresh_app();
        assert!(!get(&app));
        set(&mut app, true);
        assert!(get(&app));
    }

    #[test]
    fn add_enum_records_variants() {
        let reg = SettingsRegistry::<TestApp>::new().add_enum(
            "mode",
            get_label,
            set_label,
            &["off", "on"],
        );
        let entry = &reg.entries()[0];
        let SettingKind::Enum { variants, .. } = entry.kind else {
            panic!("expected Enum variant");
        };
        assert_eq!(variants, &["off", "on"]);
    }

    #[test]
    fn with_bounds_attaches_to_most_recent_int() {
        let reg = SettingsRegistry::<TestApp>::new()
            .add_int("count", get_int, set_int)
            .with_bounds(0, 10);
        let entry = &reg.entries()[0];
        let SettingKind::Int { bounds, .. } = entry.kind else {
            panic!("expected Int variant");
        };
        assert_eq!(bounds, Some((0, 10)));
    }

    #[test]
    fn with_bounds_no_op_when_last_is_not_int() {
        let reg = SettingsRegistry::<TestApp>::new()
            .add_bool("vim", get_bool, set_bool)
            .with_bounds(0, 10);
        let entry = &reg.entries()[0];
        assert!(matches!(entry.kind, SettingKind::Bool { .. }));
    }

    #[test]
    fn add_int_default_bounds_is_none() {
        let reg = SettingsRegistry::<TestApp>::new().add_int("count", get_int, set_int);
        let entry = &reg.entries()[0];
        let SettingKind::Int { bounds, .. } = entry.kind else {
            panic!("expected Int variant");
        };
        assert!(bounds.is_none());
    }

    #[test]
    fn entries_preserve_insertion_order() {
        let reg = SettingsRegistry::<TestApp>::new()
            .add_bool("a", get_bool, set_bool)
            .add_int("b", get_int, set_int)
            .add_enum("c", get_label, set_label, &["x"]);
        let names: Vec<&str> = reg.entries().iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
