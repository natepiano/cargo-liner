//! An active cover immediately before the hook subject merges its contribution.
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;

/// The lane claim and cover observation, followed by the future-dated contribution.
pub(crate) const COVERED_BRANCH_HOOK: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       env!("CARGO_BIN_EXE_cargo-berth"),
        name:             "covered-branch-hook",
        build:            |root| {
            IntegrationRepository::build_covered_branch_hook(
                root,
                env!("CARGO_BIN_EXE_cargo-berth"),
            );
        },
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};
