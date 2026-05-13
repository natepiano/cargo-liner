use tui_pane::ACTIVITY_SPINNER;
use tui_pane::Icon;

use crate::constants::LINT_FAILED;
use crate::constants::LINT_NO_LOG;
use crate::constants::LINT_PASSED;
use crate::constants::LINT_STALE;
use crate::lint::LintStatusKind;

/// Map a display-agnostic [`LintStatusKind`] to the concrete
/// `tui_pane::Icon` rendered in the project list. Keeps `lint/` free
/// of UI-framework imports — this is the only place the domain enum
/// crosses into `tui_pane` types.
pub const fn icon_for(kind: LintStatusKind) -> Icon {
    match kind {
        LintStatusKind::Running => Icon::Animated(ACTIVITY_SPINNER),
        LintStatusKind::Passed => Icon::Static(LINT_PASSED),
        LintStatusKind::Failed => Icon::Static(LINT_FAILED),
        LintStatusKind::Stale => Icon::Static(LINT_STALE),
        LintStatusKind::NoLog => Icon::Static(LINT_NO_LOG),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn running_kind_uses_framework_activity_spinner() {
        let elapsed = Duration::from_millis(100);

        assert_eq!(
            icon_for(LintStatusKind::Running).frame_at(elapsed),
            ACTIVITY_SPINNER.frame_at(elapsed)
        );
    }
}
