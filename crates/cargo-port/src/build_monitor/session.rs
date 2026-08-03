//! Build-session identity, scope attribution, and the immutable session
//! records the monitor pane renders.

use std::path::PathBuf;

use super::classify::BuildClassificationCycle;
use super::classify::CargoSubcommandRecognition;
use super::classify::RecognizedCargoRoot;
use super::constants::DEBUG_BUILD_DIRECTORY;
use super::constants::DEV_PROFILE;
use super::constants::RELEASE_PROFILE;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::identity::ProcessIncarnation;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalTargetDirectory;
use crate::tui::OwnedRunId;

/// Stable identity of one build session.
///
/// The inner incarnation stays private and is never handed back out: a session
/// key must not be reusable as a `ProcessIdentity`, or a same-PID exec would
/// collide two sessions in any identity-keyed map. Termination authority comes
/// from the separate signaling-authority value, never from this key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BuildSessionId(ProcessIncarnation);

impl BuildSessionId {
    /// Key a session directly from an incarnation, for tests that need a
    /// session key without a full classification cycle.
    #[cfg(test)]
    pub(super) const fn for_test(incarnation: ProcessIncarnation) -> Self { Self(incarnation) }

    /// Key a session from the validated Cargo root that established it.
    pub(crate) fn from_recognized_root(recognized_cargo_root: &RecognizedCargoRoot) -> Self {
        Self(recognized_cargo_root.incarnation().clone())
    }
}

/// Which method resolved a session's scope.
///
/// Every resolution method is actionable; only [`Self::Unresolved`] is not. The
/// method is recorded so a misattribution is diagnosable from the pane and so
/// one method can be demoted later without restructuring the record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopeAttribution {
    /// The root is the verified Cargo Port-owned run's own Cargo process.
    OwnedRoot,
    /// A `--manifest-path` or absolute manifest argument named the workspace.
    ManifestArgument,
    /// The working directory's nearest containing manifest named the workspace.
    WorkingDirectoryManifest,
    /// Exactly one indexed workspace writes to the observed output directory.
    UniqueOutputDirectory,
    /// No method related this Cargo process to anything in the project list.
    Unresolved,
}

impl ScopeAttribution {
    /// Whether the scope resolved by any method.
    pub(crate) const fn is_resolved(self) -> bool { !matches!(self, Self::Unresolved) }
}

/// The canonical checkout a resolved session builds in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionScopeRoot {
    /// The session resolved to this canonical checkout root.
    Checkout(CanonicalCheckoutRoot),
    /// No checkout root was resolved for this session.
    Unresolved,
}

/// How a session's Cargo target directory was determined.
///
/// Process observation reads argv, working directory, and ancestry but not the
/// environment, so `CARGO_TARGET_DIR` and `build.target-dir` are invisible. A
/// session whose target directory is [`Self::Unobservable`] therefore takes no
/// part in output-directory matching in either direction: it is neither
/// resolvable by unique output directory nor eligible as a cache-daemon
/// parentage candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionTargetDirectory {
    /// Read from `--target-dir` on the observed command line.
    Argument(CanonicalTargetDirectory),
    /// Assumed to be the indexed workspace's own target directory.
    Indexed(CanonicalTargetDirectory),
    /// Neither argv nor the index determines where this session writes.
    Unobservable,
}

impl SessionTargetDirectory {
    /// The directory this session writes to, when observation determined one.
    pub(crate) const fn evidence(&self) -> TargetDirectoryEvidence<'_> {
        match self {
            Self::Argument(canonical_target_directory)
            | Self::Indexed(canonical_target_directory) => {
                TargetDirectoryEvidence::Determined(canonical_target_directory)
            },
            Self::Unobservable => TargetDirectoryEvidence::Unobservable,
        }
    }
}

/// Whether a session's output location is usable for matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetDirectoryEvidence<'a> {
    /// Observation determined where this session writes.
    Determined(&'a CanonicalTargetDirectory),
    /// Observation cannot determine where this session writes.
    Unobservable,
}

/// The Cargo profile label a session builds, preserving custom names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildProfileLabel {
    /// Cargo's built-in `dev` profile.
    Dev,
    /// Cargo's built-in `release` profile.
    Release,
    /// A profile named in the manifest or on the command line.
    Custom(String),
}

impl BuildProfileLabel {
    /// The build directory Cargo writes under for this profile. Only `dev`
    /// differs from its label.
    pub(crate) fn build_directory_name(&self) -> &str {
        match self {
            Self::Dev => DEBUG_BUILD_DIRECTORY,
            Self::Release => RELEASE_PROFILE,
            Self::Custom(label) => label,
        }
    }

    /// Recognize a profile named on the command line or in metadata.
    pub(crate) fn from_label(label: &str) -> Self {
        match label {
            DEV_PROFILE => Self::Dev,
            RELEASE_PROFILE => Self::Release,
            label => Self::Custom(label.to_owned()),
        }
    }

    /// Recognize a profile from the build directory a compiler wrote under.
    /// Only `debug` differs from its profile label.
    pub(crate) fn from_build_directory_name(build_directory_name: &str) -> Self {
        match build_directory_name {
            DEBUG_BUILD_DIRECTORY => Self::Dev,
            RELEASE_PROFILE => Self::Release,
            build_directory_name => Self::Custom(build_directory_name.to_owned()),
        }
    }
}

/// Which evidence named a session's profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildProfileAttribution {
    /// An explicit `--profile` or `--release` argument.
    Argument,
    /// The build directory observed in a compiler `--out-dir`.
    OutputDirectory,
    /// Cargo's default profile for the session's subcommand.
    MetadataDefault,
}

/// A session's resolved profile and the evidence that named it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildProfile {
    label:       BuildProfileLabel,
    attribution: BuildProfileAttribution,
}

impl BuildProfile {
    /// Pair a profile label with the evidence that named it.
    pub(crate) const fn new(
        label: BuildProfileLabel,
        attribution: BuildProfileAttribution,
    ) -> Self {
        Self { label, attribution }
    }

    /// The profile label, with custom names preserved.
    pub(crate) const fn label(&self) -> &BuildProfileLabel { &self.label }

    /// Which evidence named this profile.
    pub(crate) const fn attribution(&self) -> BuildProfileAttribution { self.attribution }
}

/// Whether a Cargo Port-owned root is still live enough for its compiler
/// descendants to be running.
///
/// Live and stopping are the same fact for classification — a stopping run's
/// compiler descendants are still live — so they are one field rather than two
/// variants with identical payloads. The later signaling phases read this
/// field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRootLifecycle {
    /// The owned run is running and has not been asked to stop.
    Live,
    /// Termination was requested; the root has not yet reported completion.
    Stopping,
}

/// Immutable evidence about the one Cargo Port-owned Cargo root.
///
/// The verified identity and launch directory cross into classification as
/// observation. The opaque termination capability stays behind in the owned-run
/// aggregate and cannot be reached from here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveOwnedRoot {
    owned_run_id:         OwnedRunId,
    root_identity:        ProcessIdentity,
    launch_directory:     PathBuf,
    owned_root_lifecycle: OwnedRootLifecycle,
}

impl LiveOwnedRoot {
    /// Record the owned root's classification evidence.
    pub(crate) const fn new(
        owned_run_id: OwnedRunId,
        root_identity: ProcessIdentity,
        launch_directory: PathBuf,
        owned_root_lifecycle: OwnedRootLifecycle,
    ) -> Self {
        Self {
            owned_run_id,
            root_identity,
            launch_directory,
            owned_root_lifecycle,
        }
    }

    /// The current owned-run lifecycle identity.
    pub(crate) const fn owned_run_id(&self) -> OwnedRunId { self.owned_run_id }

    /// The verified live or stopping root process identity.
    pub(crate) const fn root_identity(&self) -> &ProcessIdentity { &self.root_identity }

    /// The directory the owned run was launched from.
    pub(crate) fn launch_directory(&self) -> &std::path::Path { &self.launch_directory }

    /// Whether the owned root is running or stopping.
    pub(crate) const fn owned_root_lifecycle(&self) -> OwnedRootLifecycle {
        self.owned_root_lifecycle
    }
}

/// Whether Cargo Port currently owns a live Cargo root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRootEvidence {
    /// No owned run has a live or stopping Cargo root.
    NoLiveRoot,
    /// One owned run's Cargo root is live or stopping.
    Root(LiveOwnedRoot),
}

/// One classified Cargo build session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildSession {
    session_id:                   BuildSessionId,
    cargo_subcommand_recognition: CargoSubcommandRecognition,
    scope_attribution:            ScopeAttribution,
    session_scope_root:           SessionScopeRoot,
    session_target_directory:     SessionTargetDirectory,
    build_profile:                BuildProfile,
    first_seen:                   BuildClassificationCycle,
}

impl BuildSession {
    /// Assemble one immutable session record.
    pub(crate) const fn new(
        build_session_id: BuildSessionId,
        cargo_subcommand_recognition: CargoSubcommandRecognition,
        scope_attribution: ScopeAttribution,
        session_scope_root: SessionScopeRoot,
        session_target_directory: SessionTargetDirectory,
        build_profile: BuildProfile,
        first_seen: BuildClassificationCycle,
    ) -> Self {
        Self {
            session_id: build_session_id,
            cargo_subcommand_recognition,
            scope_attribution,
            session_scope_root,
            session_target_directory,
            build_profile,
            first_seen,
        }
    }

    /// This session's stable key.
    pub(crate) const fn build_session_id(&self) -> &BuildSessionId { &self.session_id }

    /// How the session's Cargo subcommand was recognized.
    pub(crate) const fn cargo_subcommand_recognition(&self) -> CargoSubcommandRecognition {
        self.cargo_subcommand_recognition
    }

    /// Which method resolved this session's scope.
    pub(crate) const fn scope_attribution(&self) -> ScopeAttribution { self.scope_attribution }

    /// The canonical checkout this session builds in.
    pub(crate) const fn session_scope_root(&self) -> &SessionScopeRoot { &self.session_scope_root }

    /// Where this session writes, and how that was determined.
    pub(crate) const fn session_target_directory(&self) -> &SessionTargetDirectory {
        &self.session_target_directory
    }

    /// The profile this session builds.
    pub(crate) const fn build_profile(&self) -> &BuildProfile { &self.build_profile }

    /// The cycle in which this session was first observed.
    pub(crate) const fn first_seen(&self) -> BuildClassificationCycle { self.first_seen }
}
