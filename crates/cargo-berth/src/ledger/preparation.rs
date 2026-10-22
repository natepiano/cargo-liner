//! Optimistic mutation preparation with a serialized fallback under sustained contention.

use std::hash::BuildHasher;
use std::path::Path;
use std::time::Duration;

use super::constants::MUTATION_PREPARATION_ATTEMPTS;
use super::constants::MUTATION_RETRY_WINDOW_BASE_MILLISECONDS;
use super::error::LedgerError;
use super::lock::MutationLock;

/// Whether preparation can race another writer or already owns the mutation lock.
enum MutationPreparation {
    /// Prepare outside the lock, then check the source before publication.
    Optimistic { attempt: u32 },
    /// Prepare while holding the lock; no peer can invalidate the result.
    Locked(MutationLock),
}

/// Return a prepared value and its held mutation lock, reusing `state` across attempts.
///
/// `still_current` runs only for optimistic preparation. After
/// [`MUTATION_PREPARATION_ATTEMPTS`] invalidations, `prepare` runs under the lock and must not
/// acquire it again. A lock or preparation error still stops the operation.
pub(super) fn prepare_with_lock<State, Prepared, Failure>(
    lock_path: &Path,
    contention_tolerance: Duration,
    mut state: State,
    mut prepare: impl FnMut(&mut State) -> Result<Prepared, Failure>,
    mut still_current: impl FnMut(&mut State, &mut Prepared) -> Result<bool, Failure>,
) -> Result<(MutationLock, State, Prepared), Failure>
where
    Failure: From<LedgerError>,
{
    let retry_jitter = std::collections::hash_map::RandomState::new();
    let mut preparation = MutationPreparation::Optimistic { attempt: 0 };
    loop {
        match preparation {
            MutationPreparation::Optimistic { attempt } => {
                if attempt > 0 {
                    let window = MUTATION_RETRY_WINDOW_BASE_MILLISECONDS.pow(attempt);
                    let delay = window + retry_jitter.hash_one(attempt) % window;
                    std::thread::sleep(Duration::from_millis(delay));
                }
                let mut prepared = prepare(&mut state)?;
                let lock = MutationLock::acquire(lock_path, contention_tolerance)
                    .map_err(LedgerError::from)?;
                if still_current(&mut state, &mut prepared)? {
                    return Ok((lock, state, prepared));
                }
                std::mem::drop(lock);
                preparation = if attempt + 1 < MUTATION_PREPARATION_ATTEMPTS {
                    MutationPreparation::Optimistic {
                        attempt: attempt + 1,
                    }
                } else {
                    MutationPreparation::Locked(
                        MutationLock::acquire(lock_path, contention_tolerance)
                            .map_err(LedgerError::from)?,
                    )
                };
            },
            MutationPreparation::Locked(lock) => {
                let prepared = prepare(&mut state)?;
                return Ok((lock, state, prepared));
            },
        }
    }
}
