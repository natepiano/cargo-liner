//! Captured cargo progress, root discovery, and retained registration readings.

pub(crate) mod capture;
pub(crate) mod capture_diagnostic;
pub(crate) mod capture_read;
pub(crate) mod capture_roots;
pub(crate) mod registered_runs;

/// Cargo's count of the work in front of it, as its progress bar reports
/// it: units finished out of units planned.
///
/// A unit is one compilation of one crate target, which is what the
/// build is actually made of -- not a package and not a source file. A
/// unit already fresh counts as finished the moment cargo checks it, so
/// an incremental build opens near its total rather than at zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Progress {
    /// Units cargo has finished.
    pub(crate) done:  usize,
    /// Units in the build plan.
    pub(crate) total: usize,
}

impl Progress {
    /// How far along, rounded down, so only a finished build reads 100.
    pub(crate) const fn percent(self) -> usize {
        // `total` is never zero: `parse_counter` rejects a counter that
        // would divide by it.
        self.done.saturating_mul(100) / self.total
    }

    /// The same reading in tenths of a percent, rounded down the same
    /// way, so only a finished build reaches 1000.
    pub(crate) const fn percent_tenths(self) -> usize {
        self.done.saturating_mul(1000) / self.total
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn percent_rounds_down_so_only_a_finished_build_reads_full() {
        assert_eq!(
            Progress {
                done:  402,
                total: 403,
            }
            .percent(),
            99
        );
        assert_eq!(
            Progress {
                done:  403,
                total: 403,
            }
            .percent(),
            100
        );
    }
}
