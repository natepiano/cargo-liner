use strum::EnumIter;
use tui_pane::ToastId;

use super::dismiss_target::DismissTarget;
use super::panes::PaneId;

/// Result of a single pane's hit-test at a screen position.
#[derive(Clone, Debug)]
pub enum HoverTarget {
    PaneRow { pane: PaneId, row: usize },
    Dismiss(DismissTarget),
    ToastCard(ToastId),
}

/// Compile-time enumeration of every tiled pane that implements
/// `tui_pane::Hittable`.
#[derive(EnumIter, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HittableId {
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
}

/// Stacking order used for tiled-pane hit-test dispatch: top of stack first.
pub const HITTABLE_Z_ORDER: [HittableId; 9] = [
    HittableId::ProjectList,
    HittableId::Package,
    HittableId::Lang,
    HittableId::Cpu,
    HittableId::Git,
    HittableId::Targets,
    HittableId::Lints,
    HittableId::CiRuns,
    HittableId::Output,
];

#[cfg(test)]
mod hit_test_tests {
    use std::collections::HashSet;

    use strum::IntoEnumIterator;

    use super::HITTABLE_Z_ORDER;
    use super::HittableId;

    #[test]
    fn z_order_covers_every_hittable_id() {
        let in_order: HashSet<HittableId> = HITTABLE_Z_ORDER.iter().copied().collect();
        let all: HashSet<HittableId> = HittableId::iter().collect();
        assert_eq!(
            in_order, all,
            "every HittableId must appear exactly once in HITTABLE_Z_ORDER"
        );
        assert_eq!(
            HITTABLE_Z_ORDER.len(),
            in_order.len(),
            "HITTABLE_Z_ORDER must not contain duplicates"
        );
    }
}
