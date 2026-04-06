use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use crate::config::NavigationKeys;

// ── Key representation ───────────────────────────────────────────────

/// A bindable key: a `KeyCode` plus modifier flags from crossterm.
///
/// `=` and `+` are normalised to a single canonical form (`+`) so they
/// are treated as the same physical key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyBind {
    pub code:      KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBind {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        // BackTab implies Shift — normalise to Tab + SHIFT.
        // Uppercase Char implies Shift — strip SHIFT since it's
        // encoded in the character itself (`Char('R')` already means
        // Shift+r).  This ensures the binding `"R"` matches the
        // crossterm event `Char('R') + SHIFT`.
        // Normalise Shift + lowercase letter → uppercase letter with
        // SHIFT stripped, so `Shift+r` and `R` produce the same KeyBind.
        let (code, modifiers) = match code {
            KeyCode::BackTab => (code, modifiers | KeyModifiers::SHIFT),
            KeyCode::Char(c)
                if c.is_ascii_lowercase() && modifiers.contains(KeyModifiers::SHIFT) =>
            {
                (
                    KeyCode::Char(c.to_ascii_uppercase()),
                    modifiers - KeyModifiers::SHIFT,
                )
            },
            KeyCode::Char(c) if c.is_ascii_uppercase() => (code, modifiers - KeyModifiers::SHIFT),
            _ => (code, modifiers),
        };
        Self {
            code: normalize_code(code),
            modifiers,
        }
    }

    pub fn plain(code: KeyCode) -> Self { Self::new(code, KeyModifiers::NONE) }

    /// Human-readable glyph string for display in status bar / keymap UI.
    pub fn display(&self) -> String {
        let mut parts = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push('⌃');
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push('⌥');
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push('⇧');
        }
        parts.push_str(&code_label(self.code));
        parts
    }

    /// TOML-serialisable string (e.g. `"Ctrl+r"`, `"Shift+Tab"`, `"q"`).
    pub fn to_toml_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        parts.push(code_label(self.code));
        parts.join("+")
    }
}

impl fmt::Display for KeyBind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.display()) }
}

impl FromStr for KeyBind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> { parse_keybind(s) }
}

/// Canonical forms:
/// - `KeyCode::Char('=')` for the `=`/`+` physical key
/// - `KeyCode::Tab` for `BackTab` (Shift is added to modifiers)
const fn normalize_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char('+') => KeyCode::Char('='),
        KeyCode::BackTab => KeyCode::Tab,
        other => other,
    }
}

fn code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char('=') => "+".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab | KeyCode::BackTab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        _ => format!("{code:?}"),
    }
}

fn parse_keybind(s: &str) -> Result<KeyBind, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty key string".to_string());
    }

    // Bare "+" is the plus/equals key, not a modifier separator.
    if s == "+" || s == "=" {
        return Ok(KeyBind::plain(KeyCode::Char('+')));
    }

    let parts: Vec<&str> = s.split('+').collect();

    // Single-character key with no modifiers: e.g. "q", "/", "-"
    if parts.len() == 1 {
        let code = parse_key_code(parts[0])?;
        return Ok(KeyBind::new(code, KeyModifiers::NONE));
    }

    // Last part is the key, preceding parts are modifiers.
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);
    let key_part = key_part[0];

    if key_part.is_empty() {
        return Err(format!("modifier with no key: \"{s}\""));
    }

    let mut modifiers = KeyModifiers::NONE;
    for modifier in modifier_parts {
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => return Err(format!("unknown modifier: \"{other}\"")),
        }
    }

    let code = parse_key_code(key_part)?;
    Ok(KeyBind::new(code, modifiers))
}

fn parse_key_code(s: &str) -> Result<KeyCode, String> {
    // Named keys (case-insensitive).
    match s.to_lowercase().as_str() {
        "enter" | "return" => return Ok(KeyCode::Enter),
        "esc" | "escape" => return Ok(KeyCode::Esc),
        "tab" => return Ok(KeyCode::Tab),
        "backspace" => return Ok(KeyCode::Backspace),
        "delete" | "del" => return Ok(KeyCode::Delete),
        "home" => return Ok(KeyCode::Home),
        "end" => return Ok(KeyCode::End),
        "up" => return Ok(KeyCode::Up),
        "down" => return Ok(KeyCode::Down),
        "left" => return Ok(KeyCode::Left),
        "right" => return Ok(KeyCode::Right),
        "pageup" => return Ok(KeyCode::PageUp),
        "pagedown" => return Ok(KeyCode::PageDown),
        "space" => return Ok(KeyCode::Char(' ')),
        _ => {},
    }

    // F-keys: "F1" .. "F12".
    if let Some(n) = s.strip_prefix('F').or_else(|| s.strip_prefix('f'))
        && let Ok(n) = n.parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Ok(KeyCode::F(n));
    }

    // Single character.
    let mut chars = s.chars();
    if let Some(c) = chars.next()
        && chars.next().is_none()
    {
        return Ok(KeyCode::Char(c));
    }

    Err(format!("unknown key: \"{s}\""))
}

// ── Action enums ─────────────────────────────────────────────────────

macro_rules! action_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Name:ident {
            $( $Variant:ident => $toml_key:literal, $desc:literal; )*
        }
    ) => {
        $(#[$meta])*
        $vis enum $Name {
            $( $Variant, )*
        }

        impl $Name {
            pub const ALL: &[Self] = &[ $( Self::$Variant, )* ];

            pub const fn toml_key(self) -> &'static str {
                match self {
                    $( Self::$Variant => $toml_key, )*
                }
            }

            pub const fn description(self) -> &'static str {
                match self {
                    $( Self::$Variant => $desc, )*
                }
            }

            pub fn from_toml_key(key: &str) -> Option<Self> {
                match key {
                    $( $toml_key => Some(Self::$Variant), )*
                    _ => None,
                }
            }
        }
    };
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum GlobalAction {
        Quit       => "quit",        "Quit application";
        Restart    => "restart",     "Restart application";
        Find       => "find",        "Open finder";
        Settings   => "settings",    "Open settings";
        NextPane   => "next_pane",   "Focus next pane";
        PrevPane   => "prev_pane",   "Focus previous pane";
        FocusList  => "focus_list",  "Focus project list";
        OpenKeymap => "open_keymap", "Open keymap";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum ProjectListAction {
        OpenEditor  => "open_editor",  "Open in editor";
        ExpandAll   => "expand_all",   "Expand all";
        CollapseAll => "collapse_all", "Collapse all";
        Rescan      => "rescan",       "Rescan projects";
        Clean       => "clean",        "Clean project";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum PackageAction {
        Activate => "activate", "Open URL or Cargo.toml";
        Clean    => "clean",    "Clean project";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum GitAction {
        Activate => "activate", "Open git URL";
        Clean    => "clean",    "Clean project";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TargetsAction {
        Activate     => "activate",      "Run in debug mode";
        ReleaseBuild => "release_build", "Run in release mode";
        Clean        => "clean",         "Clean project";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum CiRunsAction {
        Activate    => "activate",     "Open run or fetch more";
        ClearCache  => "clear_cache",  "Clear CI cache";
        TogglePanel => "toggle_panel", "Switch CI/Lints panel";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum LintsAction {
        Activate     => "activate",      "Open lint output";
        ClearHistory => "clear_history", "Clear lint history";
        TogglePanel  => "toggle_panel",  "Switch CI/Lints panel";
    }
}

action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum ToastsAction {
        Dismiss => "dismiss", "Dismiss toast";
    }
}

// ── Scope map ────────────────────────────────────────────────────────

/// Bidirectional map for a single scope: key→action for dispatch,
/// action→key for display.
#[derive(Clone, Debug)]
pub struct ScopeMap<A: Copy + Eq + std::hash::Hash> {
    pub by_key:    HashMap<KeyBind, A>,
    pub by_action: HashMap<A, KeyBind>,
}

impl<A: Copy + Eq + std::hash::Hash> ScopeMap<A> {
    pub fn new() -> Self {
        Self {
            by_key:    HashMap::new(),
            by_action: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: KeyBind, action: A) {
        self.by_key.insert(key.clone(), action);
        self.by_action.insert(action, key);
    }

    pub fn action_for(&self, key: &KeyBind) -> Option<A> { self.by_key.get(key).copied() }

    pub fn key_for(&self, action: A) -> Option<&KeyBind> { self.by_action.get(&action) }

    /// Display string for an action's bound key, or `"—"` if unbound.
    pub fn display_key_for(&self, action: A) -> String {
        self.key_for(action)
            .map_or_else(|| "—".to_string(), KeyBind::display)
    }
}

impl<A: Copy + Eq + std::hash::Hash> Default for ScopeMap<A> {
    fn default() -> Self { Self::new() }
}

// ── Resolved keymap ──────────────────────────────────────────────────

/// Runtime lookup structure: one `ScopeMap` per scope, built from the
/// TOML config at load time.
#[derive(Clone, Debug, Default)]
pub struct ResolvedKeymap {
    pub global:       ScopeMap<GlobalAction>,
    pub project_list: ScopeMap<ProjectListAction>,
    pub package:      ScopeMap<PackageAction>,
    pub git:          ScopeMap<GitAction>,
    pub targets:      ScopeMap<TargetsAction>,
    pub ci_runs:      ScopeMap<CiRunsAction>,
    pub lints:        ScopeMap<LintsAction>,
    pub toasts:       ScopeMap<ToastsAction>,
}

impl ResolvedKeymap {
    /// The built-in default keymap matching the current hardcoded bindings.
    pub fn defaults() -> Self {
        let mut km = Self::default();

        // Global
        km.global
            .insert(KeyBind::plain(KeyCode::Char('q')), GlobalAction::Quit);
        km.global
            .insert(KeyBind::plain(KeyCode::Char('R')), GlobalAction::Restart);
        km.global
            .insert(KeyBind::plain(KeyCode::Char('/')), GlobalAction::Find);
        km.global
            .insert(KeyBind::plain(KeyCode::Char('s')), GlobalAction::Settings);
        km.global
            .insert(KeyBind::plain(KeyCode::Tab), GlobalAction::NextPane);
        km.global.insert(
            KeyBind::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            GlobalAction::PrevPane,
        );
        km.global
            .insert(KeyBind::plain(KeyCode::Esc), GlobalAction::FocusList);
        km.global.insert(
            KeyBind::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            GlobalAction::OpenKeymap,
        );

        // Project list
        km.project_list.insert(
            KeyBind::plain(KeyCode::Enter),
            ProjectListAction::OpenEditor,
        );
        km.project_list.insert(
            KeyBind::plain(KeyCode::Char('=')),
            ProjectListAction::ExpandAll,
        );
        km.project_list.insert(
            KeyBind::plain(KeyCode::Char('-')),
            ProjectListAction::CollapseAll,
        );
        km.project_list.insert(
            KeyBind::plain(KeyCode::Char('r')),
            ProjectListAction::Rescan,
        );
        km.project_list
            .insert(KeyBind::plain(KeyCode::Char('c')), ProjectListAction::Clean);

        // Package
        km.package
            .insert(KeyBind::plain(KeyCode::Enter), PackageAction::Activate);
        km.package
            .insert(KeyBind::plain(KeyCode::Char('c')), PackageAction::Clean);

        // Git
        km.git
            .insert(KeyBind::plain(KeyCode::Enter), GitAction::Activate);
        km.git
            .insert(KeyBind::plain(KeyCode::Char('c')), GitAction::Clean);

        // Targets
        km.targets
            .insert(KeyBind::plain(KeyCode::Enter), TargetsAction::Activate);
        km.targets.insert(
            KeyBind::plain(KeyCode::Char('r')),
            TargetsAction::ReleaseBuild,
        );
        km.targets
            .insert(KeyBind::plain(KeyCode::Char('c')), TargetsAction::Clean);

        // CI runs
        km.ci_runs
            .insert(KeyBind::plain(KeyCode::Enter), CiRunsAction::Activate);
        km.ci_runs
            .insert(KeyBind::plain(KeyCode::Char('c')), CiRunsAction::ClearCache);
        km.ci_runs.insert(
            KeyBind::plain(KeyCode::Char('p')),
            CiRunsAction::TogglePanel,
        );

        // Lints
        km.lints
            .insert(KeyBind::plain(KeyCode::Enter), LintsAction::Activate);
        km.lints.insert(
            KeyBind::plain(KeyCode::Char('c')),
            LintsAction::ClearHistory,
        );
        km.lints
            .insert(KeyBind::plain(KeyCode::Char('p')), LintsAction::TogglePanel);

        // Toasts
        km.toasts
            .insert(KeyBind::plain(KeyCode::Char('x')), ToastsAction::Dismiss);

        km
    }

    /// Generate the default TOML content for `keymap.toml`.
    pub fn default_toml() -> String {
        fn write_scope<A: Copy + Eq + std::hash::Hash>(
            out: &mut String,
            header: &str,
            scope: &ScopeMap<A>,
            actions: &[A],
            toml_key: fn(A) -> &'static str,
        ) {
            let _ = writeln!(out, "[{header}]");
            for &action in actions {
                let key_str = scope
                    .key_for(action)
                    .map_or_else(String::new, KeyBind::to_toml_string);
                let _ = writeln!(out, "{} = \"{key_str}\"", toml_key(action));
            }
            out.push('\n');
        }

        let km = Self::defaults();
        let mut out = String::from(
            "# cargo-port keymap configuration\n\
             # Edit bindings below. Format: action = \"Key\" or \"Modifier+Key\"\n\
             # Modifiers: Ctrl, Alt, Shift.  Examples: \"Ctrl+r\", \"Shift+Tab\", \"q\"\n\
             # Note: = and + are treated as the same physical key.\n\
             # Note: when vim navigation is enabled, h/j/k/l are reserved\n\
             #       for navigation and cannot be used as action keys.\n\n",
        );

        write_scope(
            &mut out,
            "global",
            &km.global,
            GlobalAction::ALL,
            GlobalAction::toml_key,
        );
        write_scope(
            &mut out,
            "project_list",
            &km.project_list,
            ProjectListAction::ALL,
            ProjectListAction::toml_key,
        );
        write_scope(
            &mut out,
            "package",
            &km.package,
            PackageAction::ALL,
            PackageAction::toml_key,
        );
        write_scope(
            &mut out,
            "git",
            &km.git,
            GitAction::ALL,
            GitAction::toml_key,
        );
        write_scope(
            &mut out,
            "targets",
            &km.targets,
            TargetsAction::ALL,
            TargetsAction::toml_key,
        );
        write_scope(
            &mut out,
            "ci_runs",
            &km.ci_runs,
            CiRunsAction::ALL,
            CiRunsAction::toml_key,
        );
        write_scope(
            &mut out,
            "lints",
            &km.lints,
            LintsAction::ALL,
            LintsAction::toml_key,
        );
        write_scope(
            &mut out,
            "toasts",
            &km.toasts,
            ToastsAction::ALL,
            ToastsAction::toml_key,
        );

        out
    }

    /// Generate TOML content from the given keymap (for saving after UI edits).
    pub fn default_toml_from(km: &Self) -> String {
        fn write_scope<A: Copy + Eq + std::hash::Hash>(
            out: &mut String,
            header: &str,
            scope: &ScopeMap<A>,
            actions: &[A],
            toml_key: fn(A) -> &'static str,
        ) {
            let _ = writeln!(out, "[{header}]");
            for &action in actions {
                let key_str = scope
                    .key_for(action)
                    .map_or_else(String::new, KeyBind::to_toml_string);
                let _ = writeln!(out, "{} = \"{key_str}\"", toml_key(action));
            }
            out.push('\n');
        }

        let mut out = String::new();
        write_scope(
            &mut out,
            "global",
            &km.global,
            GlobalAction::ALL,
            GlobalAction::toml_key,
        );
        write_scope(
            &mut out,
            "project_list",
            &km.project_list,
            ProjectListAction::ALL,
            ProjectListAction::toml_key,
        );
        write_scope(
            &mut out,
            "package",
            &km.package,
            PackageAction::ALL,
            PackageAction::toml_key,
        );
        write_scope(
            &mut out,
            "git",
            &km.git,
            GitAction::ALL,
            GitAction::toml_key,
        );
        write_scope(
            &mut out,
            "targets",
            &km.targets,
            TargetsAction::ALL,
            TargetsAction::toml_key,
        );
        write_scope(
            &mut out,
            "ci_runs",
            &km.ci_runs,
            CiRunsAction::ALL,
            CiRunsAction::toml_key,
        );
        write_scope(
            &mut out,
            "lints",
            &km.lints,
            LintsAction::ALL,
            LintsAction::toml_key,
        );
        write_scope(
            &mut out,
            "toasts",
            &km.toasts,
            ToastsAction::ALL,
            ToastsAction::toml_key,
        );
        out
    }
}

// ── Loading & validation ─────────────────────────────────────────────

pub struct KeymapLoadResult {
    pub keymap: ResolvedKeymap,
    pub errors: Vec<KeymapError>,
}

pub struct KeymapError {
    pub scope:  String,
    pub action: String,
    pub key:    String,
    pub reason: KeymapErrorReason,
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}: \"{}\" — {}",
            self.scope, self.action, self.key, self.reason
        )
    }
}

pub enum KeymapErrorReason {
    ParseError(String),
    ConflictWithGlobal(String),
    ConflictWithinScope(String),
    ReservedForVimMode,
    UnknownAction,
}

impl fmt::Display for KeymapErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
            Self::ConflictWithGlobal(action) => write!(f, "conflicts with global.{action}"),
            Self::ConflictWithinScope(action) => write!(f, "conflicts with {action}"),
            Self::ReservedForVimMode => write!(f, "reserved for vim navigation"),
            Self::UnknownAction => write!(f, "unknown action (ignored)"),
        }
    }
}

/// Path to the keymap config file.
pub fn keymap_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| {
        d.join(crate::constants::APP_NAME)
            .join(crate::constants::KEYMAP_FILE)
    })
}

/// Load and validate keymap from disk. Creates the default file if missing.
pub fn load_keymap(vim_mode: NavigationKeys) -> KeymapLoadResult {
    let Some(path) = keymap_path() else {
        return KeymapLoadResult {
            keymap: ResolvedKeymap::defaults(),
            errors: Vec::new(),
        };
    };

    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, ResolvedKeymap::default_toml());
        return KeymapLoadResult {
            keymap: ResolvedKeymap::defaults(),
            errors: Vec::new(),
        };
    }

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return KeymapLoadResult {
                keymap: ResolvedKeymap::defaults(),
                errors: vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::ParseError(format!("read error: {e}")),
                }],
            };
        },
    };

    let table: toml::Table = match contents.parse() {
        Ok(t) => t,
        Err(e) => {
            return KeymapLoadResult {
                keymap: ResolvedKeymap::defaults(),
                errors: vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::ParseError(format!("TOML parse error: {e}")),
                }],
            };
        },
    };

    resolve_from_table(&table, vim_mode)
}

/// Load keymap from a TOML string (for testing and hot-reload).
pub fn load_keymap_from_str(toml_str: &str, vim_mode: NavigationKeys) -> KeymapLoadResult {
    let table: toml::Table = match toml_str.parse() {
        Ok(t) => t,
        Err(e) => {
            return KeymapLoadResult {
                keymap: ResolvedKeymap::defaults(),
                errors: vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::ParseError(format!("TOML parse error: {e}")),
                }],
            };
        },
    };
    resolve_from_table(&table, vim_mode)
}

/// Check whether enabling vim mode would conflict with current keymap bindings.
/// Returns the list of conflicting bindings (scope.action = key).
pub fn vim_mode_conflicts(keymap: &ResolvedKeymap) -> Vec<String> {
    fn check_scope<A: Copy + Eq + std::hash::Hash>(
        scope_name: &str,
        scope: &ScopeMap<A>,
        vim_keys: &[KeyCode; 4],
        toml_key: fn(A) -> &'static str,
        conflicts: &mut Vec<String>,
    ) {
        for (bind, &action) in &scope.by_key {
            if bind.modifiers == KeyModifiers::NONE && vim_keys.contains(&bind.code) {
                conflicts.push(format!("{scope_name}.{}", toml_key(action)));
            }
        }
    }

    let vim_keys: [KeyCode; 4] = [
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
    ];
    let mut conflicts = Vec::new();

    check_scope(
        "global",
        &keymap.global,
        &vim_keys,
        GlobalAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "project_list",
        &keymap.project_list,
        &vim_keys,
        ProjectListAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "package",
        &keymap.package,
        &vim_keys,
        PackageAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "git",
        &keymap.git,
        &vim_keys,
        GitAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "targets",
        &keymap.targets,
        &vim_keys,
        TargetsAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "ci_runs",
        &keymap.ci_runs,
        &vim_keys,
        CiRunsAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "lints",
        &keymap.lints,
        &vim_keys,
        LintsAction::toml_key,
        &mut conflicts,
    );
    check_scope(
        "toasts",
        &keymap.toasts,
        &vim_keys,
        ToastsAction::toml_key,
        &mut conflicts,
    );

    conflicts
}

// ── Internal resolution ──────────────────────────────────────────────

const VIM_RESERVED: [KeyCode; 4] = [
    KeyCode::Char('h'),
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::Char('l'),
];

fn is_vim_reserved(bind: &KeyBind, vim_mode: NavigationKeys) -> bool {
    vim_mode.uses_vim() && bind.modifiers == KeyModifiers::NONE && VIM_RESERVED.contains(&bind.code)
}

fn resolve_from_table(table: &toml::Table, vim_mode: NavigationKeys) -> KeymapLoadResult {
    let defaults = ResolvedKeymap::defaults();
    let mut keymap = ResolvedKeymap::default();
    let mut errors = Vec::new();
    let no_globals = HashMap::new();

    // Phase 1: resolve globals (with intra-scope duplicate check).
    let mut ctx = ScopeResolveContext {
        table,
        errors: &mut errors,
        global_keys: &no_globals,
        vim_mode,
    };
    resolve_scope(
        &mut ctx,
        "global",
        GlobalAction::ALL,
        GlobalAction::from_toml_key,
        GlobalAction::toml_key,
        &defaults.global,
        &mut keymap.global,
    );

    // Phase 2: resolve each pane scope against the accepted globals.
    let global_keys: HashMap<KeyBind, String> = keymap
        .global
        .by_key
        .iter()
        .map(|(k, &a)| (k.clone(), a.toml_key().to_string()))
        .collect();
    ctx.global_keys = &global_keys;
    resolve_pane_scopes(&mut ctx, &defaults, &mut keymap);

    KeymapLoadResult { keymap, errors }
}

fn resolve_pane_scopes(
    ctx: &mut ScopeResolveContext<'_>,
    defaults: &ResolvedKeymap,
    keymap: &mut ResolvedKeymap,
) {
    resolve_scope(
        ctx,
        "project_list",
        ProjectListAction::ALL,
        ProjectListAction::from_toml_key,
        ProjectListAction::toml_key,
        &defaults.project_list,
        &mut keymap.project_list,
    );
    resolve_scope(
        ctx,
        "package",
        PackageAction::ALL,
        PackageAction::from_toml_key,
        PackageAction::toml_key,
        &defaults.package,
        &mut keymap.package,
    );
    resolve_scope(
        ctx,
        "git",
        GitAction::ALL,
        GitAction::from_toml_key,
        GitAction::toml_key,
        &defaults.git,
        &mut keymap.git,
    );
    resolve_scope(
        ctx,
        "targets",
        TargetsAction::ALL,
        TargetsAction::from_toml_key,
        TargetsAction::toml_key,
        &defaults.targets,
        &mut keymap.targets,
    );
    resolve_scope(
        ctx,
        "ci_runs",
        CiRunsAction::ALL,
        CiRunsAction::from_toml_key,
        CiRunsAction::toml_key,
        &defaults.ci_runs,
        &mut keymap.ci_runs,
    );
    resolve_scope(
        ctx,
        "lints",
        LintsAction::ALL,
        LintsAction::from_toml_key,
        LintsAction::toml_key,
        &defaults.lints,
        &mut keymap.lints,
    );
    resolve_scope(
        ctx,
        "toasts",
        ToastsAction::ALL,
        ToastsAction::from_toml_key,
        ToastsAction::toml_key,
        &defaults.toasts,
        &mut keymap.toasts,
    );
}

struct ScopeResolveContext<'a> {
    table:       &'a toml::Table,
    errors:      &'a mut Vec<KeymapError>,
    global_keys: &'a HashMap<KeyBind, String>,
    vim_mode:    NavigationKeys,
}

fn resolve_scope<A: Copy + Eq + std::hash::Hash>(
    ctx: &mut ScopeResolveContext<'_>,
    scope_name: &str,
    all_actions: &[A],
    from_toml_key: fn(&str) -> Option<A>,
    to_toml_key: fn(A) -> &'static str,
    defaults: &ScopeMap<A>,
    target: &mut ScopeMap<A>,
) {
    let scope_table = ctx.table.get(scope_name).and_then(toml::Value::as_table);

    // Report unknown keys in this scope.
    if let Some(st) = scope_table {
        for key in st.keys() {
            if from_toml_key(key).is_none() {
                ctx.errors.push(KeymapError {
                    scope:  scope_name.to_string(),
                    action: key.clone(),
                    key:    String::new(),
                    reason: KeymapErrorReason::UnknownAction,
                });
            }
        }
    }

    // Resolve each action.
    for &action in all_actions {
        let toml_key = to_toml_key(action);
        let raw_value = scope_table
            .and_then(|st| st.get(toml_key))
            .and_then(toml::Value::as_str);

        let bind_result = raw_value.map(str::parse::<KeyBind>);

        let (bind, error) = match bind_result {
            Some(Ok(bind)) => {
                // Validate the parsed binding.
                if is_vim_reserved(&bind, ctx.vim_mode) {
                    (None, Some(KeymapErrorReason::ReservedForVimMode))
                } else if let Some(global_action) = ctx.global_keys.get(&bind) {
                    (
                        None,
                        Some(KeymapErrorReason::ConflictWithGlobal(global_action.clone())),
                    )
                } else if let Some(&existing) = target.by_key.get(&bind) {
                    (
                        None,
                        Some(KeymapErrorReason::ConflictWithinScope(
                            to_toml_key(existing).to_string(),
                        )),
                    )
                } else {
                    (Some(bind), None)
                }
            },
            Some(Err(e)) => (None, Some(KeymapErrorReason::ParseError(e))),
            None => (None, None), // key missing from TOML — use default silently
        };

        if let Some(reason) = error {
            ctx.errors.push(KeymapError {
                scope: scope_name.to_string(),
                action: toml_key.to_string(),
                key: raw_value.unwrap_or("").to_string(),
                reason,
            });
        }

        if let Some(bind) = bind {
            target.insert(bind, action);
        } else {
            // Fall back to default binding.
            if let Some(default_bind) = defaults.key_for(action) {
                target.insert(default_bind.clone(), action);
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_char() {
        let kb: KeyBind = "q".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('q'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!("Enter".parse::<KeyBind>().unwrap().code, KeyCode::Enter);
        assert_eq!("Esc".parse::<KeyBind>().unwrap().code, KeyCode::Esc);
        assert_eq!("Tab".parse::<KeyBind>().unwrap().code, KeyCode::Tab);
        assert_eq!("Space".parse::<KeyBind>().unwrap().code, KeyCode::Char(' '));
        assert_eq!("F1".parse::<KeyBind>().unwrap().code, KeyCode::F(1));
        assert_eq!("F12".parse::<KeyBind>().unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn parse_ctrl_modifier() {
        let kb: KeyBind = "Ctrl+r".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('r'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_shift_modifier() {
        let kb: KeyBind = "Shift+Tab".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Tab);
        assert!(kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_alt_modifier() {
        let kb: KeyBind = "Alt+d".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('d'));
        assert!(kb.modifiers.contains(KeyModifiers::ALT));
    }

    #[test]
    fn parse_multiple_modifiers() {
        // Shift+x normalizes to Char('X') with SHIFT stripped.
        let kb: KeyBind = "Ctrl+Shift+x".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('X'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
        assert!(!kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn serde_round_trip() {
        let cases = [
            "q",
            "Ctrl+r",
            "Alt+d",
            "Shift+Tab",
            "Enter",
            "Esc",
            "/",
            "-",
        ];
        for input in cases {
            let kb: KeyBind = input.parse().unwrap();
            let serialized = kb.to_toml_string();
            let reparsed: KeyBind = serialized.parse().unwrap();
            assert_eq!(kb, reparsed, "round-trip failed for \"{input}\"");
        }
    }

    #[test]
    fn equals_plus_normalization() {
        let plus: KeyBind = "+".parse().unwrap();
        let equals: KeyBind = "=".parse().unwrap();
        assert_eq!(plus, equals);
    }

    #[test]
    fn uppercase_char_strips_shift() {
        // Crossterm delivers Shift+R as Char('R') + SHIFT.
        // Our normalization strips SHIFT since uppercase encodes it.
        let from_event = KeyBind::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        let from_toml = KeyBind::plain(KeyCode::Char('R'));
        assert_eq!(from_event, from_toml);
        assert_eq!(from_event.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn shift_plus_lowercase_becomes_uppercase() {
        // TOML "Shift+r" should match bare "R".
        let shift_r: KeyBind = "Shift+r".parse().unwrap();
        let bare_r: KeyBind = "R".parse().unwrap();
        assert_eq!(shift_r, bare_r);
        assert_eq!(shift_r.code, KeyCode::Char('R'));
        assert_eq!(shift_r.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn ctrl_shift_letter_keeps_ctrl() {
        // Ctrl+Shift+r → Char('R') + CONTROL (SHIFT stripped).
        let kb = KeyBind::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(kb.code, KeyCode::Char('R'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
        assert!(!kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn lowercase_without_shift_unchanged() {
        let kb = KeyBind::plain(KeyCode::Char('r'));
        assert_eq!(kb.code, KeyCode::Char('r'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn restart_default_matches_crossterm_event() {
        // The default keymap binds restart to Char('R') with NONE modifiers.
        // Crossterm sends Char('R') with SHIFT. They must match.
        let default_bind = ResolvedKeymap::defaults()
            .global
            .key_for(GlobalAction::Restart)
            .unwrap()
            .clone();
        let crossterm_event = KeyBind::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        assert_eq!(default_bind, crossterm_event);
    }

    #[test]
    fn display_glyphs() {
        assert_eq!(
            KeyBind::new(KeyCode::Char('r'), KeyModifiers::CONTROL).display(),
            "⌃r"
        );
        assert_eq!(
            KeyBind::new(KeyCode::Char('d'), KeyModifiers::ALT).display(),
            "⌥d"
        );
        assert_eq!(
            KeyBind::new(KeyCode::Tab, KeyModifiers::SHIFT).display(),
            "⇧Tab"
        );
        assert_eq!(KeyBind::plain(KeyCode::Char('q')).display(), "q");
    }

    #[test]
    fn plus_displays_as_plus() {
        let kb = KeyBind::plain(KeyCode::Char('='));
        assert_eq!(kb.display(), "+");
        assert_eq!(kb.to_toml_string(), "+");
    }

    #[test]
    fn parse_errors() {
        assert!("".parse::<KeyBind>().is_err(), "empty string");
        assert!("Ctrl+".parse::<KeyBind>().is_err(), "modifier with no key");
        assert!("Ctrl+Ctrl".parse::<KeyBind>().is_err(), "modifier as key");
    }

    #[test]
    fn valid_edge_cases() {
        assert!("+".parse::<KeyBind>().is_ok(), "plus key");
        assert!("/".parse::<KeyBind>().is_ok(), "slash key");
        assert!("Space".parse::<KeyBind>().is_ok(), "space key");
    }

    #[test]
    fn defaults_scope_map_consistency() {
        fn check<A: Copy + Eq + std::hash::Hash>(scope: &ScopeMap<A>, actions: &[A]) {
            for &action in actions {
                assert!(
                    scope.key_for(action).is_some(),
                    "action missing from by_action"
                );
            }
            for (key, &action) in &scope.by_key {
                assert_eq!(
                    scope.by_action.get(&action),
                    Some(key),
                    "by_key/by_action mismatch"
                );
            }
            assert_eq!(scope.by_key.len(), scope.by_action.len());
        }

        let km = ResolvedKeymap::defaults();
        check(&km.global, GlobalAction::ALL);
        check(&km.project_list, ProjectListAction::ALL);
        check(&km.package, PackageAction::ALL);
        check(&km.git, GitAction::ALL);
        check(&km.targets, TargetsAction::ALL);
        check(&km.ci_runs, CiRunsAction::ALL);
        check(&km.lints, LintsAction::ALL);
        check(&km.toasts, ToastsAction::ALL);
    }

    #[test]
    fn default_toml_is_parseable() {
        let toml_str = ResolvedKeymap::default_toml();
        let table: toml::Table = toml_str.parse().unwrap();
        assert!(table.contains_key("global"));
        assert!(table.contains_key("project_list"));
        assert!(table.contains_key("package"));
        assert!(table.contains_key("git"));
        assert!(table.contains_key("targets"));
        assert!(table.contains_key("ci_runs"));
        assert!(table.contains_key("lints"));
        assert!(table.contains_key("toasts"));
    }

    // ── Validation tests ─────────────────────────────────────────────

    #[test]
    fn default_toml_loads_without_errors() {
        let toml_str = ResolvedKeymap::default_toml();
        let result = load_keymap_from_str(&toml_str, NavigationKeys::ArrowsOnly);
        assert!(
            result.errors.is_empty(),
            "errors: {:?}",
            result
                .errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn global_global_conflict_detected() {
        let toml = r#"
[global]
quit = "q"
restart = "q"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ConflictWithinScope(_))),
            "expected intra-scope conflict for duplicate 'q'"
        );
    }

    #[test]
    fn pane_global_conflict_detected() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"

[project_list]
rescan = "q"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ConflictWithGlobal(_))),
            "expected conflict with global 'q'"
        );
    }

    #[test]
    fn cross_scope_same_key_is_ok() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"

[project_list]
clean = "c"

[ci_runs]
clear_cache = "c"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            !result
                .errors
                .iter()
                .any(|e| !matches!(e.reason, KeymapErrorReason::UnknownAction)),
            "unexpected errors"
        );
    }

    #[test]
    fn vim_mode_reservation() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"

[project_list]
rescan = "h"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsAndVim);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ReservedForVimMode)),
            "expected vim reservation error for 'h'"
        );
    }

    #[test]
    fn vim_mode_allows_modified_hjkl() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+h"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsAndVim);
        assert!(
            !result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ReservedForVimMode)),
            "Ctrl+h should be allowed even with vim mode"
        );
    }

    #[test]
    fn unknown_action_reported() {
        let toml = r#"
[project_list]
claen = "c"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        let unknown: Vec<_> = result
            .errors
            .iter()
            .filter(|e| matches!(e.reason, KeymapErrorReason::UnknownAction))
            .collect();
        assert!(
            !unknown.is_empty(),
            "expected unknown action for typo 'claen'"
        );
        assert_eq!(unknown[0].action, "claen");
    }

    #[test]
    fn partial_acceptance_valid_bindings_applied() {
        let toml = r#"
[global]
quit = "x"
restart = "x"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        // quit = "x" should be accepted
        assert_eq!(
            result.keymap.global.key_for(GlobalAction::Quit),
            Some(&KeyBind::plain(KeyCode::Char('x')))
        );
        // restart = "x" conflicts with quit, should fall back to default
        assert!(
            result
                .keymap
                .global
                .key_for(GlobalAction::Restart)
                .is_some(),
            "restart should have a fallback binding"
        );
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn malformed_toml_returns_defaults() {
        let result = load_keymap_from_str("{{invalid toml", NavigationKeys::ArrowsOnly);
        assert!(!result.errors.is_empty());
        // Should have defaults for all actions.
        assert!(result.keymap.global.key_for(GlobalAction::Quit).is_some());
    }

    #[test]
    fn vim_mode_conflicts_detected() {
        let defaults = ResolvedKeymap::defaults();
        let conflicts = vim_mode_conflicts(&defaults);
        // Default keymap doesn't use bare hjkl.
        assert!(conflicts.is_empty());

        // Build a keymap with 'h' bound.
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "h"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+k"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        let conflicts = vim_mode_conflicts(&result.keymap);
        assert!(!conflicts.is_empty(), "expected conflict for 'h' binding");
    }

    #[test]
    fn action_description_and_display_key() {
        assert_eq!(GlobalAction::Quit.description(), "Quit application");
        let km = ResolvedKeymap::defaults();
        assert_eq!(km.global.display_key_for(GlobalAction::Quit), "q");
        assert_eq!(km.global.display_key_for(GlobalAction::OpenKeymap), "⌃k");
    }
}
