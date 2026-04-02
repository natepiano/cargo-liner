/// The current input context, derived from app state. Determines which
/// shortcuts are shown in the status bar and how keys are dispatched.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) enum InputContext {
    ProjectList,
    ScanLog,
    DetailFields,
    DetailTargets,
    CiRuns,
    Searching,
    Finder,
    Settings,
}

impl InputContext {
    /// Text-input contexts consume all `Char` keys, so global shortcuts
    /// (which are letter-based) must not be shown or dispatched.
    pub const fn is_text_input(self) -> bool {
        matches!(self, Self::Searching | Self::Finder | Self::Settings)
    }
}

/// A keyboard shortcut for display in the status bar.
pub(super) struct Shortcut {
    pub key:         &'static str,
    pub description: &'static str,
}

// ── Reusable shortcut definitions ──────────────────────────────────────

const NAV: Shortcut = Shortcut {
    key:         "↑/↓",
    description: "nav",
};
const ARROWS_COLUMN: Shortcut = Shortcut {
    key:         "←/→",
    description: "column",
};
const ARROWS_EXPAND: Shortcut = Shortcut {
    key:         "←/→",
    description: "expand",
};
const ARROWS_TOGGLE: Shortcut = Shortcut {
    key:         "←/→",
    description: "toggle",
};
const TAB_PANE: Shortcut = Shortcut {
    key:         "Tab",
    description: "pane",
};
const ESC_BACK: Shortcut = Shortcut {
    key:         "Esc",
    description: "back",
};
const ESC_CANCEL: Shortcut = Shortcut {
    key:         "Esc",
    description: "cancel",
};
const ESC_CLOSE: Shortcut = Shortcut {
    key:         "Esc",
    description: "close",
};
const RELEASE: Shortcut = Shortcut {
    key:         "r",
    description: "release",
};
const CLEAN: Shortcut = Shortcut {
    key:         "c",
    description: "clean",
};
const CLEAR_CACHE: Shortcut = Shortcut {
    key:         "c",
    description: "clear cache",
};
const RESCAN: Shortcut = Shortcut {
    key:         "r",
    description: "rescan",
};
const SETTINGS: Shortcut = Shortcut {
    key:         "s",
    description: "settings",
};
const FIND: Shortcut = Shortcut {
    key:         "/",
    description: "find",
};
const QUIT: Shortcut = Shortcut {
    key:         "q",
    description: "quit",
};
const RESTART: Shortcut = Shortcut {
    key:         "R",
    description: "restart",
};

const fn enter(description: &'static str) -> Shortcut {
    Shortcut {
        key: "Enter",
        description,
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/// Status bar shortcut groups: left (navigation), center (pane actions),
/// right (globals).
pub(super) struct StatusBarGroups {
    pub navigation: Vec<Shortcut>,
    pub actions:    Vec<Shortcut>,
    pub global:     Vec<Shortcut>,
}

/// Build all three shortcut groups for the current context.
pub(super) fn for_status_bar(
    context: InputContext,
    enter_action: Option<&'static str>,
    is_rust: bool,
) -> StatusBarGroups {
    let (navigation, actions) = match context {
        InputContext::Searching => (vec![NAV], vec![enter("select"), ESC_CANCEL]),
        InputContext::Finder => (vec![NAV], vec![enter("go to"), ESC_CLOSE]),
        InputContext::Settings => (vec![NAV, ARROWS_TOGGLE], vec![enter("edit"), ESC_CLOSE]),
        InputContext::DetailFields | InputContext::DetailTargets => {
            detail_groups(context, enter_action, is_rust)
        },
        InputContext::CiRuns => ci_groups(enter_action),
        InputContext::ScanLog | InputContext::ProjectList => {
            project_list_groups(enter_action, is_rust)
        },
    };

    let global = if context.is_text_input() {
        vec![]
    } else {
        vec![FIND, SETTINGS, QUIT, RESTART]
    };

    StatusBarGroups {
        navigation,
        actions,
        global,
    }
}

// ── Context builders ───────────────────────────────────────────────────

fn detail_groups(
    context: InputContext,
    enter_action: Option<&'static str>,
    is_rust: bool,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, ARROWS_COLUMN, TAB_PANE, ESC_BACK];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if context == InputContext::DetailTargets {
        actions.push(RELEASE);
    }
    if is_rust {
        actions.push(CLEAN);
    }

    (navigation, actions)
}

fn ci_groups(enter_action: Option<&'static str>) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, TAB_PANE, ESC_BACK];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    actions.push(CLEAR_CACHE);

    (navigation, actions)
}

fn project_list_groups(
    enter_action: Option<&'static str>,
    is_rust: bool,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, ARROWS_EXPAND, TAB_PANE];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if is_rust {
        actions.push(CLEAN);
    }
    actions.push(RESCAN);

    (navigation, actions)
}
