use crate::tui::pane::PaneAxisSize;
use crate::tui::pane::PaneSizeSpec;

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub enum PaneId {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneBehavior {
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

pub const fn behavior(id: PaneId) -> PaneBehavior {
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

pub const fn size_spec(id: PaneId, cpu_width: u16) -> PaneSizeSpec {
    match id {
        PaneId::Cpu => PaneSizeSpec {
            width:  PaneAxisSize::Fixed(cpu_width),
            height: PaneAxisSize::Fill(1),
        },
        _ => PaneSizeSpec::fill(),
    }
}

#[cfg(test)]
mod tests {
    //! Characterization tests pinning the current `behavior` /
    //! `size_spec` mappings.
    use super::*;

    fn all_pane_ids() -> [PaneId; 13] {
        [
            PaneId::ProjectList,
            PaneId::Package,
            PaneId::Lang,
            PaneId::Cpu,
            PaneId::Git,
            PaneId::Targets,
            PaneId::Lints,
            PaneId::CiRuns,
            PaneId::Output,
            PaneId::Toasts,
            PaneId::Settings,
            PaneId::Finder,
            PaneId::Keymap,
        ]
    }

    #[test]
    fn behavior_mapping_is_pinned() {
        assert_eq!(behavior(PaneId::ProjectList), PaneBehavior::ProjectList);
        assert_eq!(behavior(PaneId::Package), PaneBehavior::DetailFields);
        assert_eq!(behavior(PaneId::Lang), PaneBehavior::DetailFields);
        assert_eq!(behavior(PaneId::Git), PaneBehavior::DetailFields);
        assert_eq!(behavior(PaneId::Cpu), PaneBehavior::Cpu);
        assert_eq!(behavior(PaneId::Targets), PaneBehavior::DetailTargets);
        assert_eq!(behavior(PaneId::Lints), PaneBehavior::Lints);
        assert_eq!(behavior(PaneId::CiRuns), PaneBehavior::CiRuns);
        assert_eq!(behavior(PaneId::Output), PaneBehavior::Output);
        assert_eq!(behavior(PaneId::Toasts), PaneBehavior::Toasts);
        assert_eq!(behavior(PaneId::Settings), PaneBehavior::Overlay);
        assert_eq!(behavior(PaneId::Finder), PaneBehavior::Overlay);
        assert_eq!(behavior(PaneId::Keymap), PaneBehavior::Overlay);
    }

    #[test]
    fn size_spec_cpu_takes_fixed_width_others_fill() {
        let cpu = size_spec(PaneId::Cpu, 12);
        assert!(matches!(cpu.width, PaneAxisSize::Fixed(12)));
        assert!(matches!(cpu.height, PaneAxisSize::Fill(1)));
        for id in all_pane_ids()
            .into_iter()
            .filter(|id| !matches!(id, PaneId::Cpu))
        {
            let spec = size_spec(id, 12);
            assert_eq!(
                spec,
                PaneSizeSpec::fill(),
                "{id:?} should use the default fill size spec"
            );
        }
    }

    #[test]
    fn size_spec_cpu_width_threads_through() {
        for w in [4, 12, 32, 80] {
            let spec = size_spec(PaneId::Cpu, w);
            assert!(matches!(spec.width, PaneAxisSize::Fixed(actual) if actual == w));
        }
    }
}
