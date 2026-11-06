//! Parse launcher flags before their legacy positional arguments.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::DispatchRole;
use super::JobSource;
use super::JobText;
use super::NotWorkerDispatch;
use super::SeatSelection;
use super::WorkerRole;
use crate::constants::DISPATCH_CLAIM_PID_FLAG;
use crate::constants::DISPATCH_CWD_FLAG;
use crate::constants::DISPATCH_DIFF_FLAG;
use crate::constants::DISPATCH_EARLY_READY_FLAG;
use crate::constants::DISPATCH_EFFORT_FLAG;
use crate::constants::DISPATCH_FAMILY_FLAG;
use crate::constants::DISPATCH_FIX_CAUSE_FLAG;
use crate::constants::DISPATCH_FROM_FLAG;
use crate::constants::DISPATCH_FROM_SESSION_DIR_FLAG;
use crate::constants::DISPATCH_GUARD_TEXT_FLAG;
use crate::constants::DISPATCH_HELD_DESIGN_FLAG;
use crate::constants::DISPATCH_HELD_ROLE_FLAG;
use crate::constants::DISPATCH_HUMAN_FLAG;
use crate::constants::DISPATCH_ID_FILE_FLAG;
use crate::constants::DISPATCH_IMAGE_FLAG;
use crate::constants::DISPATCH_INSTRUCTIONS_FILE_FLAG;
use crate::constants::DISPATCH_LEASE_FLAG;
use crate::constants::DISPATCH_LENS_FLAG;
use crate::constants::DISPATCH_LOG_FILE_FLAG;
use crate::constants::DISPATCH_LOG_FLAG;
use crate::constants::DISPATCH_MODEL_FLAG;
use crate::constants::DISPATCH_NAME_FLAG;
use crate::constants::DISPATCH_NEW_WORKER_FLAG;
use crate::constants::DISPATCH_NO_ACCOUNTING_FLAG;
use crate::constants::DISPATCH_OPERATION_FLAG;
use crate::constants::DISPATCH_PASS_FLAG;
use crate::constants::DISPATCH_PATHS_FLAG;
use crate::constants::DISPATCH_PHASE_FLAG;
use crate::constants::DISPATCH_POOL_FLAG;
use crate::constants::DISPATCH_PROMPT_FILE_FLAG;
use crate::constants::DISPATCH_PROMPT_FLAG;
use crate::constants::DISPATCH_READ_ONLY_FLAG;
use crate::constants::DISPATCH_REPEAT_CHECKED_FLAG;
use crate::constants::DISPATCH_REPLY_FILE_FLAG;
use crate::constants::DISPATCH_REPORT_RECIPIENT_FLAG;
use crate::constants::DISPATCH_RESIDENT_FLAG;
use crate::constants::DISPATCH_ROLE_FLAG;
use crate::constants::DISPATCH_SANDBOX_FLAG;
use crate::constants::DISPATCH_SERVICE_TIER_FLAG;
use crate::constants::DISPATCH_SLOT_FLAG;
use crate::constants::DISPATCH_STAFF_ID_FLAG;
use crate::constants::DISPATCH_STYLE_SINCE_FLAG;
use crate::constants::DISPATCH_STYLE_UNTIL_FLAG;
use crate::constants::DISPATCH_SUBMISSION_RECEIPT_FILE_FLAG;
use crate::constants::DISPATCH_SUMMARY_FILE_FLAG;
use crate::constants::DISPATCH_SUMMARY_FLAG;
use crate::constants::DISPATCH_TASK_FLAG;
use crate::constants::DISPATCH_TIMEOUT_FLAG;
use crate::constants::MESSAGE_FILE_FLAG;
use crate::constants::MESSAGE_FLAG;
use crate::constants::SEAT_FLAG;
use crate::constants::SESSION_DIRECTORY_FLAG;

/// The positional arguments accepted by each launcher.
#[derive(Clone, Copy)]
pub(super) enum DispatchPositionals {
    Implement,
    Review,
    None,
}

impl DispatchPositionals {
    /// Identify a legacy argument tail before skipping an unknown flag value.
    fn accepts(self, words: &[&str]) -> bool {
        match self {
            Self::Implement => {
                matches!(words.len(), 9 | 10)
                    && matches!(words.get(3), Some(&"impl" | &"test" | &"review" | &"fix"))
            },
            Self::Review => {
                words
                    .first()
                    .is_some_and(|word| Path::new(word).is_absolute())
                    && words.len() <= 9
            },
            Self::None => words.is_empty(),
        }
    }
}

/// The number of arguments consumed by a supported launcher flag.
enum DispatchFlagArity {
    Value,
    Switch,
    Future,
}

impl From<&str> for DispatchFlagArity {
    fn from(flag: &str) -> Self {
        match flag {
            DISPATCH_READ_ONLY_FLAG
            | DISPATCH_RESIDENT_FLAG
            | DISPATCH_HUMAN_FLAG
            | DISPATCH_NO_ACCOUNTING_FLAG
            | DISPATCH_REPORT_RECIPIENT_FLAG
            | DISPATCH_REPEAT_CHECKED_FLAG => Self::Switch,
            DISPATCH_ROLE_FLAG
            | DISPATCH_OPERATION_FLAG
            | SEAT_FLAG
            | DISPATCH_NEW_WORKER_FLAG
            | DISPATCH_MODEL_FLAG
            | DISPATCH_EFFORT_FLAG
            | DISPATCH_SUMMARY_FLAG
            | DISPATCH_FIX_CAUSE_FLAG
            | DISPATCH_PATHS_FLAG
            | DISPATCH_PHASE_FLAG
            | DISPATCH_EARLY_READY_FLAG
            | SESSION_DIRECTORY_FLAG
            | DISPATCH_STYLE_SINCE_FLAG
            | DISPATCH_STYLE_UNTIL_FLAG
            | DISPATCH_HELD_DESIGN_FLAG
            | DISPATCH_LOG_FLAG
            | DISPATCH_ID_FILE_FLAG
            | DISPATCH_FROM_SESSION_DIR_FLAG
            | DISPATCH_HELD_ROLE_FLAG
            | DISPATCH_FAMILY_FLAG
            | DISPATCH_PROMPT_FLAG
            | DISPATCH_CWD_FLAG
            | DISPATCH_LENS_FLAG
            | DISPATCH_DIFF_FLAG
            | DISPATCH_PASS_FLAG
            | DISPATCH_IMAGE_FLAG
            | DISPATCH_SLOT_FLAG
            | DISPATCH_POOL_FLAG
            | DISPATCH_LEASE_FLAG
            | DISPATCH_NAME_FLAG
            | DISPATCH_STAFF_ID_FLAG
            | DISPATCH_CLAIM_PID_FLAG
            | MESSAGE_FILE_FLAG
            | DISPATCH_SUMMARY_FILE_FLAG
            | DISPATCH_REPLY_FILE_FLAG
            | DISPATCH_LOG_FILE_FLAG
            | DISPATCH_TASK_FLAG
            | DISPATCH_PROMPT_FILE_FLAG
            | DISPATCH_INSTRUCTIONS_FILE_FLAG
            | DISPATCH_SERVICE_TIER_FLAG
            | DISPATCH_SANDBOX_FLAG
            | DISPATCH_TIMEOUT_FLAG
            | DISPATCH_SUBMISSION_RECEIPT_FILE_FLAG
            | DISPATCH_FROM_FLAG
            | DISPATCH_GUARD_TEXT_FLAG
            | MESSAGE_FLAG => Self::Value,
            _ => Self::Future,
        }
    }
}

/// Launcher options retain identity when a newer launcher adds a flag.
pub(super) struct DispatchOptions<'a> {
    pub(super) values:     BTreeMap<&'a str, &'a str>,
    pub(super) positional: Vec<&'a str>,
}

impl<'a> DispatchOptions<'a> {
    pub(super) fn read(
        words: impl IntoIterator<Item = &'a str>,
        schema: DispatchPositionals,
    ) -> Result<Self, NotWorkerDispatch> {
        let words: Vec<_> = words.into_iter().collect();
        let mut options = Self {
            values:     BTreeMap::new(),
            positional: Vec::new(),
        };
        let mut index = 0;
        while let Some(word) = words.get(index) {
            index += 1;
            if !word.starts_with("--") {
                options.positional.extend_from_slice(&words[index - 1..]);
                break;
            }
            let (flag, inline) = word
                .split_once('=')
                .map_or((*word, None), |(flag, value)| (flag, Some(value)));
            match DispatchFlagArity::from(flag) {
                DispatchFlagArity::Value => {
                    let value = if let Some(value) = inline {
                        value
                    } else {
                        let value = words
                            .get(index)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or(NotWorkerDispatch)?;
                        index += 1;
                        value
                    };
                    options.values.insert(flag, value);
                },
                DispatchFlagArity::Switch => {},
                DispatchFlagArity::Future => {
                    if inline.is_none()
                        && words
                            .get(index)
                            .is_some_and(|value| !value.starts_with("--"))
                        && !schema.accepts(&words[index..])
                    {
                        index += 1;
                    }
                },
            }
        }
        if matches!(schema, DispatchPositionals::None) && !options.positional.is_empty() {
            return Err(NotWorkerDispatch);
        }
        Ok(options)
    }

    pub(super) fn required(&self, flag: &str) -> Result<&'a str, NotWorkerDispatch> {
        self.values
            .get(flag)
            .copied()
            .filter(|value| !value.is_empty())
            .ok_or(NotWorkerDispatch)
    }

    pub(super) fn seat(&self) -> SeatSelection {
        self.values
            .get(SEAT_FLAG)
            .or_else(|| self.values.get(DISPATCH_NEW_WORKER_FLAG))
            .map_or(SeatSelection::Launcher, |seat| {
                SeatSelection::Named((*seat).to_owned())
            })
    }

    pub(super) fn role(&self) -> DispatchRole {
        match self.values.get(DISPATCH_ROLE_FLAG).copied() {
            Some("implement" | "impl") => DispatchRole::Worker(WorkerRole::Impl),
            Some(role) => DispatchRole::from_short_seat(role),
            None => DispatchRole::Unroled,
        }
    }

    pub(super) fn prompt_job(&self, flag: &str) -> String {
        let prompt = self
            .values
            .get(flag)
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        match JobText::parse(&prompt, JobSource::FirstTaskPrompt) {
            JobText::Recorded(job) => job,
            JobText::Unrecorded => String::new(),
        }
    }
}
