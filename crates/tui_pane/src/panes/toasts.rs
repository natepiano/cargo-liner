crate::action_enum! {
    /// Actions reachable on the toast stack's local bar.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ToastsAction {
        /// Activate the currently focused toast when it has a payload.
        // 3-positional because bar label "open" differs from TOML key "activate".
        Activate => ("activate", "open", "Activate focused toast");
    }
}
