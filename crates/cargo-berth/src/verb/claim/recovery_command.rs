//! The parsed caller claim retained for an overlap-answer repair.

use std::path::PathBuf;

use crate::answer::OrderingReplacement;
use crate::cli::CliOutputFormat;
use crate::coordination_identity;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorkPlanPhase;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::WorkPlanReference;

/// A selector supplied by the caller, independent of the answer being repaired.
#[derive(Clone)]
pub(crate) enum ClaimRecoverySelector {
    Reservation(ReservationId),
    Target(String),
    Purpose(String),
    Plan(WorkPlanReference),
    Phase(WorkPlanPhase),
    Run(CoordinationRunId),
    Head(ProtectedPhaseStartHead),
}

impl ClaimRecoverySelector {
    fn append_arguments(&self, arguments: &mut Vec<String>) {
        let (flag, value) = match self {
            Self::Reservation(id) => ("--reservation", id.to_string()),
            Self::Target(target) => ("--target", target.clone()),
            Self::Purpose(reason) => ("--why", reason.clone()),
            Self::Plan(plan) => ("--plan", plan.to_string()),
            Self::Phase(phase) => ("--phase", phase.to_string()),
            Self::Run(run) => ("--run", run.to_string()),
            Self::Head(head) => ("--head", head.as_ref().to_string()),
        };
        if value.starts_with('-') {
            arguments.push(format!(
                "{flag}={}",
                coordination_identity::shell_quote(&value)
            ));
        } else {
            arguments.extend([flag.to_owned(), coordination_identity::shell_quote(&value)]);
        }
    }
}

/// The non-answer portions of a claim, captured from clap's parsed values.
#[derive(Clone)]
pub(crate) struct ClaimRecoveryCommand {
    pub(crate) paths:       Vec<PathBuf>,
    pub(crate) selectors:   Vec<ClaimRecoverySelector>,
    pub(crate) replacement: OrderingReplacement,
    pub(crate) output:      CliOutputFormat,
}

impl ClaimRecoveryCommand {
    pub(super) fn leading_arguments(&self) -> Vec<String> {
        let mut arguments = vec!["cargo-berth".to_owned(), "claim".to_owned()];
        if !self.paths_need_separator() {
            self.append_paths(&mut arguments);
        }
        for selector in &self.selectors {
            selector.append_arguments(&mut arguments);
        }
        arguments
    }

    pub(super) fn finish_arguments(&self, arguments: &mut Vec<String>) {
        if matches!(self.replacement, OrderingReplacement::Replace) {
            arguments.push("--replace".to_owned());
        }
        if matches!(self.output, CliOutputFormat::Json) {
            arguments.push("--json".to_owned());
        }
        if self.paths_need_separator() {
            arguments.push("--".to_owned());
            self.append_paths(arguments);
        }
    }

    fn paths_need_separator(&self) -> bool {
        self.paths
            .iter()
            .any(|path| path.as_os_str().as_encoded_bytes().starts_with(b"-"))
    }

    fn append_paths(&self, arguments: &mut Vec<String>) {
        arguments.extend(
            self.paths
                .iter()
                .map(|path| coordination_identity::shell_quote(&path.to_string_lossy())),
        );
    }
}
