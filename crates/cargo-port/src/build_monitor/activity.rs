//! Compile-activity identity, compiler-to-session attribution, and the
//! non-actionable records the monitor pane renders for evidence it cannot
//! resolve.

use cargo_metadata::PackageId;

use super::classify::BuildClassificationCycle;
use super::classify::RecognizedCompilerProcess;
use super::session::BuildSessionId;
use crate::process_observation::identity::ProcessIncarnation;

/// Stable identity of one compile activity row.
///
/// The inner incarnation stays private for the same reason it does on
/// [`BuildSessionId`]: handing it back out would let a same-PID exec collide
/// two rows in an identity-keyed map, and would turn a lookup key into
/// something that looks like signaling authority.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CompileActivityId(ProcessIncarnation);

impl CompileActivityId {
    /// Key an activity from the validated compiler process that produced it.
    pub(crate) fn from_recognized_compiler(
        recognized_compiler_process: &RecognizedCompilerProcess,
    ) -> Self {
        Self(recognized_compiler_process.incarnation().clone())
    }
}

/// Which compiler executable a compile activity runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompilerKind {
    /// `rustc`.
    Rustc,
    /// `clippy-driver`.
    ClippyDriver,
    /// `rustdoc`.
    Rustdoc,
    /// A `build-script-*` executable Cargo compiled and ran.
    BuildScript,
    /// A linker invoked beneath a compiler or build script.
    Linker,
    /// A compiler wrapper such as `sccache` standing in front of `rustc`.
    Wrapper,
}

/// Sessions that an ambiguous compile activity could belong to.
///
/// The collection is private so the [`CompilerAttribution::Ambiguous`] variant
/// can only be produced from the slice match in [`Self::attribution`]; an
/// `Ambiguous` value with zero or one candidate would restate the two other
/// variants.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompilerSessionCandidates(Vec<BuildSessionId>);

impl CompilerSessionCandidates {
    /// Record one compatible live session, ignoring repeats.
    pub(crate) fn include(&mut self, build_session_id: BuildSessionId) {
        if !self.0.contains(&build_session_id) {
            self.0.push(build_session_id);
        }
    }

    /// Convert collected candidates into attribution. Exactly one candidate
    /// resolves to `resolved_by`; zero or more than one cannot.
    pub(crate) fn attribution(
        self,
        resolved_by: ResolvedCompilerAttribution,
    ) -> CompilerAttribution {
        match self.0.as_slice() {
            [] => CompilerAttribution::Unattributed,
            [_] => {
                let mut candidates = self.0;
                candidates
                    .pop()
                    .map_or(
                        CompilerAttribution::Unattributed,
                        |build_session_id| match resolved_by {
                            ResolvedCompilerAttribution::ValidatedParentChain => {
                                CompilerAttribution::Confirmed(build_session_id)
                            },
                            ResolvedCompilerAttribution::OutputDirectoryMatch
                            | ResolvedCompilerAttribution::PrimaryInputMatch => {
                                CompilerAttribution::UniqueOutputMatch(build_session_id)
                            },
                        },
                    )
            },
            [_, _, ..] => CompilerAttribution::Ambiguous {
                candidates: Self(self.0),
            },
        }
    }

    /// Convert collected candidates into attribution for a cycle in which at
    /// least one candidate carried unreadable identity evidence. Even a single
    /// candidate is ambiguous here: the system-wide uniqueness test could not
    /// see every candidate, so the match may be false-unique and must not
    /// authorize termination.
    pub(crate) fn degraded_attribution(self) -> CompilerAttribution {
        match self.0.as_slice() {
            [] => CompilerAttribution::Unattributed,
            [_, ..] => CompilerAttribution::Ambiguous { candidates: self },
        }
    }

    /// The candidate sessions, for the attribution-unavailable section.
    pub(crate) fn candidates(&self) -> &[BuildSessionId] { &self.0 }

    /// How many compatible live sessions were collected.
    pub(crate) const fn len(&self) -> usize { self.0.len() }

    /// Whether no compatible live session was collected.
    pub(crate) const fn is_empty(&self) -> bool { self.0.is_empty() }
}

/// Which association method selected a single collected candidate.
///
/// [`Self::OutputDirectoryMatch`] and [`Self::PrimaryInputMatch`] are separate
/// methods over separate evidence — the compiler's `--out-dir` under a session's
/// target directory, and the compiler's primary input source file under a
/// session's checkout root. Both carry the same authority and both map to
/// [`CompilerAttribution::UniqueOutputMatch`]; keeping them apart here is what
/// lets a misattribution name the evidence that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedCompilerAttribution {
    /// A validated parent chain reached the session's Cargo root.
    ValidatedParentChain,
    /// The compiler's output directory sits under exactly one live session's
    /// target directory.
    OutputDirectoryMatch,
    /// The compiler's primary input resolves under exactly one live session's
    /// checkout root.
    PrimaryInputMatch,
}

/// Which session a compile activity belongs to, and on what evidence.
///
/// [`Self::UniqueOutputMatch`] authorizes termination exactly as
/// [`Self::Confirmed`] does: a target directory that exactly one live session
/// writes to is sufficient evidence of ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompilerAttribution {
    /// A validated parent chain reached this session's Cargo root.
    Confirmed(BuildSessionId),
    /// Output-directory or primary-input evidence selected exactly one live
    /// session.
    UniqueOutputMatch(BuildSessionId),
    /// More than one compatible live session claims this compiler.
    Ambiguous {
        /// The compatible live sessions, rendered once in the scope-level
        /// attribution-unavailable section.
        candidates: CompilerSessionCandidates,
    },
    /// No live session is compatible with this compiler at all.
    Unattributed,
}

impl CompilerAttribution {
    /// The session this activity is actionable against, when one resolved.
    pub(crate) const fn attributed_session(&self) -> AttributedSession<'_> {
        match self {
            Self::Confirmed(build_session_id) | Self::UniqueOutputMatch(build_session_id) => {
                AttributedSession::Session(build_session_id)
            },
            Self::Ambiguous { .. } | Self::Unattributed => AttributedSession::Unresolved,
        }
    }
}

/// Whether a compile activity resolved to one owning session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttributedSession<'a> {
    /// The activity belongs to this session.
    Session(&'a BuildSessionId),
    /// The activity belongs to no single session and is observed only.
    Unresolved,
}

/// The `--crate-name` a compiler was invoked with.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CompiledCrateName(String);

impl CompiledCrateName {
    /// The crate name as the compiler reported it.
    pub(crate) fn as_str(&self) -> &str { &self.0 }
}

impl From<String> for CompiledCrateName {
    fn from(crate_name: String) -> Self { Self(crate_name) }
}

/// Package identity read from a dependency's own manifest.
///
/// Dependencies are absent from `cargo metadata --no-deps`, so no
/// `cargo_metadata::PackageId` exists for them in the shared index.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ManifestPackageIdentity {
    name:    String,
    version: String,
}

impl ManifestPackageIdentity {
    /// Record a dependency package read from its manifest.
    pub(crate) const fn new(name: String, version: String) -> Self { Self { name, version } }

    /// The package name declared in the manifest.
    pub(crate) fn name(&self) -> &str { &self.name }

    /// The package version declared in the manifest.
    pub(crate) fn version(&self) -> &str { &self.version }
}

/// What a compile activity is known to be compiling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompiledCrateIdentity {
    /// An indexed workspace member, identified exactly.
    WorkspacePackage(PackageId),
    /// A dependency identified from its own manifest.
    DependencyPackage(ManifestPackageIdentity),
    /// Only the compiler's `--crate-name` is known so far.
    CrateNameOnly(CompiledCrateName),
}

/// One classified compile activity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompileActivity {
    activity_id:             CompileActivityId,
    compiler_kind:           CompilerKind,
    compiler_attribution:    CompilerAttribution,
    compiled_crate_identity: CompiledCrateIdentity,
    compilation_target:      CompilationTarget,
    first_seen:              BuildClassificationCycle,
}

impl CompileActivity {
    /// Assemble one immutable activity record.
    pub(crate) const fn new(
        compile_activity_id: CompileActivityId,
        compiler_kind: CompilerKind,
        compiler_attribution: CompilerAttribution,
        compiled_crate_identity: CompiledCrateIdentity,
        compilation_target: CompilationTarget,
        first_seen: BuildClassificationCycle,
    ) -> Self {
        Self {
            activity_id: compile_activity_id,
            compiler_kind,
            compiler_attribution,
            compiled_crate_identity,
            compilation_target,
            first_seen,
        }
    }

    /// This activity's stable key.
    pub(crate) const fn compile_activity_id(&self) -> &CompileActivityId { &self.activity_id }

    /// Which compiler executable this activity runs.
    pub(crate) const fn compiler_kind(&self) -> CompilerKind { self.compiler_kind }

    /// Which session this activity belongs to, and on what evidence.
    pub(crate) const fn compiler_attribution(&self) -> &CompilerAttribution {
        &self.compiler_attribution
    }

    /// What this activity is compiling.
    pub(crate) const fn compiled_crate_identity(&self) -> &CompiledCrateIdentity {
        &self.compiled_crate_identity
    }

    /// The target triple this activity compiles for.
    pub(crate) const fn compilation_target(&self) -> &CompilationTarget { &self.compilation_target }

    /// The cycle in which this activity was first observed.
    pub(crate) const fn first_seen(&self) -> BuildClassificationCycle { self.first_seen }
}

/// The target triple a compile activity builds for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompilationTarget {
    /// An explicit `--target` triple, which also appears in the output path.
    Explicit(String),
    /// No `--target` argument: the compiler builds for the host.
    Host,
}

/// One compile activity the classifier could not attribute, rendered once in
/// the scope-level attribution-unavailable section rather than under a session
/// it might not belong to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnattributedCompileActivity {
    activity_id:             CompileActivityId,
    compiler_kind:           CompilerKind,
    compiled_crate_identity: CompiledCrateIdentity,
    compiler_attribution:    CompilerAttribution,
}

impl UnattributedCompileActivity {
    /// Record one non-actionable compile activity.
    pub(crate) const fn new(
        compile_activity_id: CompileActivityId,
        compiler_kind: CompilerKind,
        compiled_crate_identity: CompiledCrateIdentity,
        compiler_attribution: CompilerAttribution,
    ) -> Self {
        Self {
            activity_id: compile_activity_id,
            compiler_kind,
            compiled_crate_identity,
            compiler_attribution,
        }
    }

    /// This activity's stable key.
    pub(crate) const fn compile_activity_id(&self) -> &CompileActivityId { &self.activity_id }

    /// Which compiler executable this activity runs.
    pub(crate) const fn compiler_kind(&self) -> CompilerKind { self.compiler_kind }

    /// What this activity is compiling, as far as argv reveals.
    pub(crate) const fn compiled_crate_identity(&self) -> &CompiledCrateIdentity {
        &self.compiled_crate_identity
    }

    /// The ambiguous or unattributed evidence, including candidate sessions.
    pub(crate) const fn compiler_attribution(&self) -> &CompilerAttribution {
        &self.compiler_attribution
    }
}
