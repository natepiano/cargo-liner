/// Release metadata from one crates.io response. Kept together through fetching,
/// project storage and rendering so metadata cannot come from another release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CratesIoRelease {
    /// Latest stable version, or the newest prerelease when no stable exists.
    pub version:    String,
    /// Newer prerelease alongside the stable version, when one exists.
    pub prerelease: Option<String>,
    pub downloads:  u64,
}

/// Whether a crates.io lookup has supplied a release for this package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum CratesIoReleaseState {
    #[default]
    Awaiting,
    Released(CratesIoRelease),
}

impl CratesIoReleaseState {
    pub(crate) fn version(&self) -> Option<&str> {
        match self {
            Self::Awaiting => None,
            Self::Released(crates_io_release) => Some(&crates_io_release.version),
        }
    }
}
