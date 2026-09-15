use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use tempfile::TempDir;

use super::mend_json::expected_summary;
use super::mend_json::mend_command;
use super::mend_json::parse_mend_json_output;
use super::report::Finding;
use super::report::Report;
use super::report::Summary;

enum FixtureLayout {
    Workspace,
    SiblingModules,
}

/// Independent cases checked together with one explicit configuration.
pub(crate) struct DiagnosticBatch {
    temp:    TempDir,
    members: Vec<String>,
    layout:  FixtureLayout,
}

impl DiagnosticBatch {
    pub(crate) fn new(config: &str) -> Self {
        let temp = tempfile::tempdir().expect("create diagnostics workspace");
        fs::write(temp.path().join("mend.toml"), config).expect("write batch configuration");
        Self {
            temp,
            members: Vec::new(),
            layout: FixtureLayout::Workspace,
        }
    }

    pub(crate) fn new_crate(config: &str) -> Self {
        let mut batch = Self::new(config);
        batch.layout = FixtureLayout::SiblingModules;
        fs::create_dir(batch.path().join("src")).expect("create batch source directory");
        fs::write(
            batch.path().join("Cargo.toml"),
            "[package]\nname = \"diagnostics_batch\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write batch crate manifest");
        batch
    }

    /// Sources are relative to `src/name`; the case's root is `mod.rs`.
    pub(crate) fn add_module(&mut self, name: &str, sources: &[(&str, &str)]) -> PathBuf {
        assert!(matches!(self.layout, FixtureLayout::SiblingModules));
        assert!(
            !self.members.iter().any(|member| member == name),
            "duplicate module {name}"
        );
        let root = self.path().join("src").join(name);
        fs::create_dir_all(&root).expect("create case module");
        for (relative, source) in sources {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("source parent"))
                .expect("create source directory");
            fs::write(path, source).expect("write module source");
        }
        self.members.push(name.to_owned());
        let declarations = format!("mod {};\nfn main() {{}}\n", self.members.join(";\nmod "));
        fs::write(self.path().join("src/main.rs"), declarations).expect("write batch crate root");
        root
    }

    /// Sources are relative to the member. Supply Cargo.toml to customize targets.
    pub(crate) fn add_member(&mut self, name: &str, sources: &[(&str, &str)]) -> PathBuf {
        assert!(matches!(self.layout, FixtureLayout::Workspace));
        assert!(
            !self.members.iter().any(|member| member == name),
            "duplicate member {name}"
        );
        let root = self.temp.path().join(name);
        fs::create_dir_all(&root).expect("create fixture member");
        fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .expect("write member manifest");
        for (relative, source) in sources {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("source parent"))
                .expect("create source directory");
            fs::write(path, source).expect("write member source");
        }
        self.members.push(name.to_owned());
        fs::write(
            self.temp.path().join("Cargo.toml"),
            format!(
                "[workspace]\nresolver = \"3\"\nmembers = {:?}\n",
                self.members
            ),
        )
        .expect("write workspace manifest");
        root
    }

    pub(crate) fn path(&self) -> &Path { self.temp.path() }

    pub(crate) fn command(&self) -> Command {
        let mut command = mend_command();
        command
            .current_dir(self.path())
            .arg("--manifest-path")
            .arg(self.path().join("Cargo.toml"))
            .arg("--workspace");
        command
    }

    pub(crate) fn report(&self) -> Report {
        let output = self
            .command()
            .arg("--json")
            .output()
            .expect("run batch diagnostics");
        assert!(
            matches!(output.status.code(), Some(0..=2)),
            "batch diagnostics failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_mend_json_output(&output.stdout)
    }

    /// Retain complete paths while restoring each case's own summary counts.
    pub(crate) fn member_reports(&self) -> BTreeMap<String, Report> {
        self.partition_report(self.report())
    }

    pub(crate) fn partition_report(&self, report: Report) -> BTreeMap<String, Report> {
        assert!(matches!(self.layout, FixtureLayout::Workspace));
        let mut reports = self
            .members
            .iter()
            .map(|member| {
                (
                    member.clone(),
                    Report {
                        findings: Vec::new(),
                        summary:  Summary {
                            errors:                   0,
                            warnings:                 0,
                            fixable_with_fix:         0,
                            fixable_with_fix_pub_use: 0,
                        },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for finding in report.findings {
            let (member, _) = finding
                .path
                .split_once('/')
                .expect("member-prefixed finding path");
            reports
                .get_mut(member)
                .expect("finding belongs to a registered member")
                .findings
                .push(finding);
        }
        for report in reports.values_mut() {
            report.summary = expected_summary(report);
        }
        reports
    }
}

/// Copy one member's findings using a directory-component prefix, retaining full paths.
pub(crate) fn member_report(report: &Report, member: &str) -> Report {
    let findings = report
        .findings
        .iter()
        .filter(|finding| Path::new(&finding.path).starts_with(member))
        .map(|finding| Finding {
            code:        finding.code,
            headline:    finding.headline.clone(),
            path:        finding.path.clone(),
            line_start:  finding.line_start,
            item:        finding.item.clone(),
            fix_support: finding.fix_support,
            help:        finding.help.clone(),
        })
        .collect();
    let mut report = Report {
        findings,
        summary: Summary {
            errors:                   0,
            warnings:                 0,
            fixable_with_fix:         0,
            fixable_with_fix_pub_use: 0,
        },
    };
    report.summary = expected_summary(&report);
    report
}

/// Match the entire reported path, including a workspace member's directory.
pub(crate) fn findings_at<'a>(report: &'a Report, path: &str) -> Vec<&'a Finding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.path == path)
        .collect()
}

pub(crate) fn assert_codes_at(report: &Report, path: &str, expected: &[DiagnosticCode]) {
    let codes = findings_at(report, path)
        .iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert_eq!(
        codes, expected,
        "unexpected diagnostic codes at {path}: {report:#?}"
    );
}

// fix notes
const NOTE_FIXABLE_WITH_FIX: &str = "this warning is auto-fixable with `cargo mend --fix`";
const NOTE_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this warning is auto-fixable with `cargo mend --fix-pub-use`";
const NOTE_ERROR_FIXABLE_WITH_FIX: &str = "this error is auto-fixable with `cargo mend --fix`";
const NOTE_ERROR_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this error is auto-fixable with `cargo mend --fix-pub-use`";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticCode {
    OverbroadPubCrate,
    ForbiddenPubInCrate,
    ReviewPubMod,
    SuspiciousPub,
    UnusedPub,
    PreferModuleImport,
    InlinePathQualifiedType,
    ShortenLocalCrateImport,
    ReplaceDeepSuperImport,
    WildcardParentPubUse,
    InternalParentPubUseFacade,
    NarrowToPubCrate,
    FieldVisibilityWiderThanType,
    ImportsAtTop,
}

impl DiagnosticCode {
    pub(crate) const ALL: &[Self] = &[
        Self::OverbroadPubCrate,
        Self::ForbiddenPubInCrate,
        Self::ReviewPubMod,
        Self::SuspiciousPub,
        Self::UnusedPub,
        Self::PreferModuleImport,
        Self::InlinePathQualifiedType,
        Self::ShortenLocalCrateImport,
        Self::ReplaceDeepSuperImport,
        Self::WildcardParentPubUse,
        Self::InternalParentPubUseFacade,
        Self::NarrowToPubCrate,
        Self::FieldVisibilityWiderThanType,
        Self::ImportsAtTop,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OverbroadPubCrate => "overbroad_pub_crate",
            Self::ForbiddenPubInCrate => "forbidden_pub_in_crate",
            Self::ReviewPubMod => "review_pub_mod",
            Self::SuspiciousPub => "suspicious_pub",
            Self::UnusedPub => "unused_pub",
            Self::PreferModuleImport => "prefer_module_import",
            Self::InlinePathQualifiedType => "inline_path_qualified_type",
            Self::ShortenLocalCrateImport => "shorten_local_crate_import",
            Self::ReplaceDeepSuperImport => "replace_deep_super_import",
            Self::WildcardParentPubUse => "wildcard_parent_pub_use",
            Self::InternalParentPubUseFacade => "internal_parent_pub_use_facade",
            Self::NarrowToPubCrate => "narrow_to_pub_crate",
            Self::FieldVisibilityWiderThanType => "field_visibility_wider_than_type",
            Self::ImportsAtTop => "imports_at_top",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FixSupport {
    #[default]
    None,
    ShortenImport,
    PreferModuleImport,
    InlinePathQualifiedType,
    #[serde(rename = "fix_pub_use")]
    PubUse,
    NeedsManualPubUseCleanup,
    InternalParentFacade,
    UnusedPub,
    NarrowToPubCrate,
    RestrictedAnnotation,
    #[serde(rename = "fix_field_visibility")]
    FieldVisibility,
    #[serde(rename = "fix_imports_at_top")]
    ImportsAtTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixSummaryBucket {
    Standard,
    PubUse,
}

impl FixSupport {
    pub(crate) const fn note(self) -> Option<&'static str> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::RestrictedAnnotation
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(NOTE_FIXABLE_WITH_FIX),
            Self::PubUse => Some(NOTE_FIXABLE_WITH_FIX_PUB_USE),
        }
    }

    pub(crate) const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::RestrictedAnnotation
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(FixSummaryBucket::Standard),
            Self::PubUse => Some(FixSummaryBucket::PubUse),
        }
    }
}

/// The `--fix` route a rendered diagnostic advertised, read back from its fix
/// note. The cargo JSON a parsed `Finding` comes from carries that note and
/// never the `FixSupport` variant behind it, so this is the whole fixability
/// fact the harness can recover: `DiagnosticSpec::fix_support` states only the
/// per-code default, and a route a single finding earns on its own — a
/// `suspicious_pub` whose bare `pub` narrows to an exact module boundary —
/// reaches the JSON as this note and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvertisedFix {
    NotOffered,
    WithFix,
    WithFixPubUse,
}

impl AdvertisedFix {
    pub(crate) fn from_notes<'note>(notes: impl IntoIterator<Item = &'note str>) -> Self {
        let mut advertised = Self::NotOffered;
        for note in notes {
            // `--fix-pub-use` is the more specific claim and wins: the two note
            // texts differ past `--fix`, so test each note against the longer
            // one first.
            if note.contains(NOTE_FIXABLE_WITH_FIX_PUB_USE)
                || note.contains(NOTE_ERROR_FIXABLE_WITH_FIX_PUB_USE)
            {
                return Self::WithFixPubUse;
            }
            if note.contains(NOTE_FIXABLE_WITH_FIX) || note.contains(NOTE_ERROR_FIXABLE_WITH_FIX) {
                advertised = Self::WithFix;
            }
        }
        advertised
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticSpec {
    pub headline:    HeadlineSource,
    pub help_anchor: &'static str,
    pub fix_support: FixSupport,
}

#[derive(Debug, Clone, Copy)]
pub enum HeadlineSource {
    Literal(&'static str),
    FindingMessage { fallback: &'static str },
}

impl HeadlineSource {
    pub const fn resolve(self, headline: &str) -> &str {
        match self {
            Self::Literal(literal) => literal,
            Self::FindingMessage { fallback } => {
                if headline.is_empty() {
                    fallback
                } else {
                    headline
                }
            },
        }
    }
}

pub(crate) const fn diagnostic_spec(code: DiagnosticCode) -> &'static DiagnosticSpec {
    const OVERBROAD_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::FindingMessage {
            fallback: "`pub(crate)` is broader than required",
        },
        help_anchor: "overbroad-pub-crate",
        fix_support: FixSupport::None,
    };
    const FORBIDDEN_PUB_IN_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::FindingMessage {
            fallback: "use of `pub(in crate::...)` is forbidden by policy",
        },
        help_anchor: "forbidden-pub-in-crate",
        fix_support: FixSupport::None,
    };
    const REVIEW_PUB_MOD: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("`pub mod` requires explicit review or allowlisting"),
        help_anchor: "review-pub-mod",
        fix_support: FixSupport::None,
    };
    const SUSPICIOUS_PUB: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("`pub` is broader than this nested module boundary"),
        help_anchor: "suspicious-pub",
        fix_support: FixSupport::None,
    };
    const UNUSED_PUB: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("`pub` item is not used outside its defining module"),
        help_anchor: "unused-pub",
        fix_support: FixSupport::UnusedPub,
    };
    const PREFER_MODULE_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("function import should use module-qualified form"),
        help_anchor: "prefer-module-import",
        fix_support: FixSupport::PreferModuleImport,
    };
    const INLINE_PATH_QUALIFIED_TYPE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal(
            "inline path-qualified type should use a `use` import",
        ),
        help_anchor: "inline-path-qualified-type",
        fix_support: FixSupport::InlinePathQualifiedType,
    };
    const SHORTEN_LOCAL_CRATE_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal(
            "crate-relative import can be shortened to a local-relative import",
        ),
        help_anchor: "shorten-local-crate-import",
        fix_support: FixSupport::ShortenImport,
    };
    const REPLACE_DEEP_SUPER_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("deep `super::` chain should use a `crate::` path"),
        help_anchor: "replace-deep-super-import",
        fix_support: FixSupport::ShortenImport,
    };
    const WILDCARD_PARENT_PUB_USE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("parent module `pub use *` should be explicit"),
        help_anchor: "wildcard-parent-pub-use",
        fix_support: FixSupport::None,
    };
    const INTERNAL_PARENT_PUB_USE_FACADE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::FindingMessage {
            fallback: "parent module re-export is acting as an internal facade",
        },
        help_anchor: "internal-parent-pub-use-facade",
        fix_support: FixSupport::InternalParentFacade,
    };
    const NARROW_TO_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal(
            "`pub` exceeds the item's effective reach — use `pub(crate)`",
        ),
        help_anchor: "narrow-to-pub-crate",
        fix_support: FixSupport::NarrowToPubCrate,
    };
    const FIELD_VISIBILITY_WIDER_THAN_TYPE: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal("field visibility is wider than its containing type"),
        help_anchor: "field-visibility-wider-than-type",
        fix_support: FixSupport::FieldVisibility,
    };
    const IMPORTS_AT_TOP: DiagnosticSpec = DiagnosticSpec {
        headline:    HeadlineSource::Literal(
            "`use` statement should live at the top of the file or inline module",
        ),
        help_anchor: "imports-at-top",
        fix_support: FixSupport::ImportsAtTop,
    };

    match code {
        DiagnosticCode::OverbroadPubCrate => &OVERBROAD_PUB_CRATE,
        DiagnosticCode::ForbiddenPubInCrate => &FORBIDDEN_PUB_IN_CRATE,
        DiagnosticCode::ReviewPubMod => &REVIEW_PUB_MOD,
        DiagnosticCode::SuspiciousPub => &SUSPICIOUS_PUB,
        DiagnosticCode::UnusedPub => &UNUSED_PUB,
        DiagnosticCode::PreferModuleImport => &PREFER_MODULE_IMPORT,
        DiagnosticCode::InlinePathQualifiedType => &INLINE_PATH_QUALIFIED_TYPE,
        DiagnosticCode::ShortenLocalCrateImport => &SHORTEN_LOCAL_CRATE_IMPORT,
        DiagnosticCode::ReplaceDeepSuperImport => &REPLACE_DEEP_SUPER_IMPORT,
        DiagnosticCode::WildcardParentPubUse => &WILDCARD_PARENT_PUB_USE,
        DiagnosticCode::InternalParentPubUseFacade => &INTERNAL_PARENT_PUB_USE_FACADE,
        DiagnosticCode::NarrowToPubCrate => &NARROW_TO_PUB_CRATE,
        DiagnosticCode::FieldVisibilityWiderThanType => &FIELD_VISIBILITY_WIDER_THAN_TYPE,
        DiagnosticCode::ImportsAtTop => &IMPORTS_AT_TOP,
    }
}
