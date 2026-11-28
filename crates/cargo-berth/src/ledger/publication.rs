//! Atomic publication of a file and its directory entry.

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;

pub(crate) fn replace(
    directory: &Path,
    path: &Path,
    temporary_path: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    let publication = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)?;
        file.write_all(bytes)?;
        #[cfg(test)]
        sync_tests::observe(path, sync_tests::SyncStep::File)?;
        file.sync_all()?;
        fs::rename(temporary_path, path)?;
        let directory = File::open(directory)?;
        #[cfg(test)]
        sync_tests::observe(path, sync_tests::SyncStep::Directory)?;
        directory.sync_all()
    })();
    if publication.is_err() {
        std::mem::drop(fs::remove_file(temporary_path));
    }
    publication
}

#[cfg(test)]
pub(crate) mod sync_tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::Path;
    use std::path::PathBuf;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum SyncStep {
        File,
        Directory,
    }

    struct SyncProbe {
        directory: PathBuf,
        fail_at:   Option<SyncStep>,
        steps:     Vec<SyncStep>,
    }

    thread_local! {
        static PROBE: RefCell<Option<SyncProbe>> = const { RefCell::new(None) };
    }

    struct RestoreProbe(Option<SyncProbe>);

    impl Drop for RestoreProbe {
        fn drop(&mut self) { PROBE.replace(self.0.take()); }
    }

    pub(crate) fn record<Output>(
        directory: &Path,
        fail_at: Option<SyncStep>,
        action: impl FnOnce() -> Output,
    ) -> (Output, Vec<SyncStep>) {
        let _restore = RestoreProbe(PROBE.replace(Some(SyncProbe {
            directory: directory.to_path_buf(),
            fail_at,
            steps: Vec::new(),
        })));
        let output = action();
        let steps = PROBE.take().map_or_else(Vec::new, |probe| probe.steps);
        (output, steps)
    }

    pub(super) fn observe(path: &Path, step: SyncStep) -> io::Result<()> {
        PROBE.with_borrow_mut(|probe| {
            if let Some(probe) = probe
                .as_mut()
                .filter(|probe| path.parent() == Some(probe.directory.as_path()))
            {
                probe.steps.push(step);
                if probe.fail_at == Some(step) {
                    return Err(io::Error::other(format!("injected {step:?} sync failure")));
                }
            }
            Ok(())
        })
    }
}
