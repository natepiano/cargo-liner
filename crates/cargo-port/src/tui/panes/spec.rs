use crate::tui::pane::PaneAxisSize;
use crate::tui::pane::PaneKey;
use crate::tui::pane::PaneSizeSpec;

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub(in super::super) enum PaneId {
    #[default]
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
    Toasts,
    Settings,
    Finder,
    Keymap,
}

impl PaneId {
    pub(in super::super) const fn index(self) -> usize {
        match self {
            Self::ProjectList => 0,
            Self::Package => 1,
            Self::Lang => 2,
            Self::Cpu => 3,
            Self::Git => 4,
            Self::Targets => 5,
            Self::Lints => 6,
            Self::CiRuns => 7,
            Self::Output => 8,
            Self::Toasts => 9,
            Self::Settings => 10,
            Self::Finder => 11,
            Self::Keymap => 12,
        }
    }

    pub(in super::super) const fn pane_count() -> usize { Self::Keymap.index() + 1 }

    pub(in super::super) const fn is_overlay(self) -> bool {
        matches!(self, Self::Settings | Self::Finder)
    }
}

impl PaneKey for PaneId {
    fn index(self) -> usize { Self::index(self) }

    fn key_count() -> usize { Self::pane_count() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum PaneBehavior {
    ProjectList,
    DetailFields,
    DetailTargets,
    Cpu,
    Lints,
    CiRuns,
    Output,
    Toasts,
    Overlay,
}

pub(in super::super) const fn behavior(id: PaneId) -> PaneBehavior {
    match id {
        PaneId::ProjectList => PaneBehavior::ProjectList,
        PaneId::Package | PaneId::Lang | PaneId::Git => PaneBehavior::DetailFields,
        PaneId::Cpu => PaneBehavior::Cpu,
        PaneId::Targets => PaneBehavior::DetailTargets,
        PaneId::Lints => PaneBehavior::Lints,
        PaneId::CiRuns => PaneBehavior::CiRuns,
        PaneId::Output => PaneBehavior::Output,
        PaneId::Toasts => PaneBehavior::Toasts,
        PaneId::Settings | PaneId::Finder | PaneId::Keymap => PaneBehavior::Overlay,
    }
}

pub(in super::super) const fn has_row_hitboxes(id: PaneId) -> bool {
    matches!(
        behavior(id),
        PaneBehavior::DetailFields | PaneBehavior::DetailTargets
    )
}

pub(in super::super) const fn size_spec(id: PaneId, cpu_width: u16) -> PaneSizeSpec {
    match id {
        PaneId::Cpu => PaneSizeSpec {
            width:  PaneAxisSize::Fixed(cpu_width),
            height: PaneAxisSize::Fill(1),
        },
        _ => PaneSizeSpec::fill(),
    }
}
