use super::CiData;
use super::GitData;
use super::LintsData;
use super::PackageData;
use super::TargetsData;
use crate::tui::cpu::CpuSnapshot;

pub(in super::super) struct PaneDataStore {
    pub(in super::super) package: Option<PackageData>,
    pub(in super::super) git:     Option<GitData>,
    pub(in super::super) cpu:     Option<CpuSnapshot>,
    pub(in super::super) targets: Option<TargetsData>,
    pub(in super::super) ci:      Option<CiData>,
    pub(in super::super) lints:   Option<LintsData>,
}

impl PaneDataStore {
    pub(in super::super) const fn new() -> Self {
        Self {
            package: None,
            git:     None,
            cpu:     None,
            targets: None,
            ci:      None,
            lints:   None,
        }
    }

    pub(in super::super) fn set_detail_data(
        &mut self,
        package: PackageData,
        git: GitData,
        targets: TargetsData,
        ci: CiData,
        lints: LintsData,
    ) {
        self.package = Some(package);
        self.git = Some(git);
        self.targets = Some(targets);
        self.ci = Some(ci);
        self.lints = Some(lints);
    }

    pub(in super::super) fn clear_detail_data(&mut self) {
        self.package = None;
        self.git = None;
        self.targets = None;
        self.ci = None;
        self.lints = None;
    }
}
