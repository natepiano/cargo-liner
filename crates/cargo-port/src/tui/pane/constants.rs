use super::HittableId;

// src tui pane mod
/// Stacking order used for tiled-pane hit-test dispatch: top of stack
/// first. Overlays and toasts are not here — see [`HittableId`].
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
