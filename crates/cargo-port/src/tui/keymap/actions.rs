use tui_pane::Action;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum ProjectListAction {
        ExpandAll   => ("expand_all",   "+", "Expand all");
        CollapseAll => ("collapse_all", "-", "Collapse all");
        ExpandRow   => ("expand_row",   "→", "Expand row");
        CollapseRow => ("collapse_row", "←", "Collapse row");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum PackageAction {
        Activate => ("activate", "Open URL or Cargo.toml");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum GitAction {
        Activate => ("activate", "Open git URL");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TargetsAction {
        Activate     => ("activate",      "run",     "Run in debug mode");
        ReleaseBuild => ("release_build", "release", "Run in release mode");
        Kill         => ("kill",          "kill",    "Kill running instance");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum CiRunsAction {
        Activate   => ("activate",    "open",        "Open run");
        FetchMore  => ("fetch_more",  "fetch more",  "Fetch more CI runs");
        ShowBranch => ("show_branch", "branch",      "Show branch-only runs");
        ShowAll    => ("show_all",    "all",         "Show all runs");
        ClearCache => ("clear_cache", "del cache", "Clear CI cache");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum LintsAction {
        Activate     => ("activate",      "open",        "Open lint output");
        ClearHistory => ("clear_history", "del history", "Clear lint history");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum OutputAction {
        Cancel => ("cancel", "close", "Close output pane");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum FinderAction {
        Activate => ("activate", "go to", "Go to selected project");
        Cancel   => ("cancel",   "close", "Close finder");
    }
}

pub(super) fn action_toml_key<A: Action>(action: A) -> &'static str { action.toml_key() }

pub(super) fn action_from_toml_key<A: Action>(key: &str) -> Option<A> { A::from_toml_key(key) }
