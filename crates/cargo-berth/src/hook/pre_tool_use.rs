//! Raw `PreToolUse` payload parsing and edit authorization.

use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::BLOCKING_EXIT_CODE;
use super::HarnessSessionIdentityAvailability;
use super::HookWorkingDirectorySelection;
use super::refuse_hook_request;
use super::render_blocks;
use super::write_stderr_line;
use crate::coordination_identity::RecoveryCommandLine;
use crate::exit::BerthExit;
use crate::ledger::AncestorCanonicalizationError;
use crate::ledger::LedgerError;
use crate::ledger::WorktreeContext;
use crate::ledger::canonicalize_through_nearest_existing_ancestor;
use crate::ledger::normalize_absolute_path;
use crate::output::LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE;
use crate::output::OutputEnvelope;
use crate::presentation::EnvelopePresentation;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::ScopeKind;
use crate::verb::check;
use crate::verb::check::CheckRequest;
use crate::verb::claim::CheckReservationSelection;

const HOOK_EVENT_NAME: &str = "PreToolUse";
const AUTHORIZED_SYSTEM_MESSAGE: &str =
    "cargo-berth authorized this edit and stated the detail below itself.";
const FAIL_OPEN_SYSTEM_MESSAGE: &str =
    "cargo-berth could not establish edit safety and stated the detail below itself.";

/// Serde-only representation of one raw harness payload.
#[derive(Deserialize)]
struct PreToolUsePayloadBoundary {
    tool_name:  Option<String>,
    tool_input: Option<PreToolUseToolInputBoundary>,
    cwd:        Option<String>,
    session_id: Option<String>,
}

/// Serde-only representation of the supported edit-path fields.
#[derive(Deserialize)]
struct PreToolUseToolInputBoundary {
    file_path:     Option<String>,
    notebook_path: Option<String>,
}

/// Whether the raw payload requests one supported file-writing authorization.
enum PreToolUseEditAuthorizationRequest {
    /// A supported edit tool supplied the remaining typed request context.
    Requested(SupportedPreToolUseEditAuthorizationRequest),
    /// The payload omitted a supported file-writing tool request.
    NotRequested { reason: String },
}

/// The typed context carried by one supported edit-authorization request.
struct SupportedPreToolUseEditAuthorizationRequest {
    payload_edit_target:             PayloadEditTarget,
    working_directory_selection:     HookWorkingDirectorySelection,
    harness_session_id_availability: HarnessSessionIdentityAvailability,
}

/// The edit path one supported request named, before repository resolution.
enum PayloadEditTarget {
    /// The payload named a path that still needs repository resolution.
    Named(PathBuf),
    /// The payload did not name a usable path for its edit request.
    NotNamed { reason: String },
}

/// Where the hook placed a named edit target relative to the coordination domain.
enum ResolvedEditTarget {
    /// The named path belongs to the repository selected by the hook working directory.
    WithinRepository {
        repository_root:          PathBuf,
        repository_relative_path: String,
    },
    /// No repository selected by the hook working directory contains the named path.
    OutsideCoordinationDomain,
    /// The hook could not establish where the named path sits, so it refuses the edit.
    Unresolved { reason: String },
}

/// A raw payload could not be converted into the semantic hook request.
enum PreToolUsePayloadParseError {
    /// Serde could not read the expected payload object and boundary field types.
    ///
    /// A payload whose `cwd` or `session_id` is present but not a string lands here and
    /// refuses the edit. The shell hook this verb replaces coerced such a value to an
    /// empty string and continued; that coercion is deliberately not restored. A safety
    /// gate that cannot understand its input must refuse rather than proceed on a coerced
    /// value, because an empty `cwd` silently selects a different repository and an empty
    /// `session_id` silently selects a different session's reservations.
    InvalidPayload,
}

impl Display for PreToolUsePayloadParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPayload => formatter.write_str(
                "stdin was invalid JSON or did not contain the expected PreToolUse fields",
            ),
        }
    }
}

impl PreToolUseEditAuthorizationRequest {
    fn from_value(value: &Value) -> Result<Self, PreToolUsePayloadParseError> {
        let boundary = serde_json::from_value::<PreToolUsePayloadBoundary>(value.clone())
            .map_err(|_| PreToolUsePayloadParseError::InvalidPayload)?;
        let Some(tool_name) = boundary.tool_name else {
            return Ok(Self::NotRequested {
                reason: "stdin did not name a supported file-writing tool".to_owned(),
            });
        };
        if !matches!(tool_name.as_str(), "Edit" | "Write" | "NotebookEdit") {
            return Ok(Self::NotRequested {
                reason: format!("unsupported tool_name {tool_name}"),
            });
        }
        Ok(Self::Requested(
            SupportedPreToolUseEditAuthorizationRequest {
                payload_edit_target:             PayloadEditTarget::from_boundary(
                    &tool_name,
                    boundary.tool_input,
                ),
                working_directory_selection:     HookWorkingDirectorySelection::from_boundary(
                    boundary.cwd,
                ),
                harness_session_id_availability: HarnessSessionIdentityAvailability::from_boundary(
                    boundary.session_id,
                ),
            },
        ))
    }
}

impl PayloadEditTarget {
    fn from_boundary(tool_name: &str, tool_input: Option<PreToolUseToolInputBoundary>) -> Self {
        let Some(tool_input) = tool_input else {
            return Self::NotNamed {
                reason: format!("{tool_name} did not supply tool_input"),
            };
        };
        let (path_field, edit_path) = match tool_name {
            "Edit" | "Write" => ("file_path", tool_input.file_path),
            "NotebookEdit" => ("notebook_path", tool_input.notebook_path),
            _ => {
                return Self::NotNamed {
                    reason: "the supported edit request lost its path field".to_owned(),
                };
            },
        };
        edit_path
            .filter(|edit_path| !edit_path.is_empty())
            .map_or_else(
                || Self::NotNamed {
                    reason: format!("{tool_name} requires a non-empty tool_input.{path_field}"),
                },
                |edit_path| Self::Named(PathBuf::from(edit_path)),
            )
    }

    fn resolve(self, working_directory: &HookWorkingDirectorySelection) -> ResolvedEditTarget {
        match self {
            Self::Named(edit_path) => resolve_named_edit_target(&edit_path, working_directory),
            Self::NotNamed { reason } => ResolvedEditTarget::Unresolved { reason },
        }
    }
}

/// Read and execute one raw `PreToolUse` edit-authorization payload.
pub(crate) fn execute() -> ExitCode {
    let request = match read_request() {
        Ok(request) => request,
        Err(error) => return refuse(&error.to_string()),
    };
    let SupportedPreToolUseEditAuthorizationRequest {
        payload_edit_target,
        working_directory_selection,
        harness_session_id_availability,
    } = match request {
        PreToolUseEditAuthorizationRequest::Requested(request) => request,
        PreToolUseEditAuthorizationRequest::NotRequested { reason } => return refuse(&reason),
    };
    let (repository_root, repository_relative_path) =
        match payload_edit_target.resolve(&working_directory_selection) {
            ResolvedEditTarget::WithinRepository {
                repository_root,
                repository_relative_path,
            } => (repository_root, repository_relative_path),
            ResolvedEditTarget::OutsideCoordinationDomain => return ExitCode::SUCCESS,
            ResolvedEditTarget::Unresolved { reason } => return refuse(&reason),
        };
    if std::env::set_current_dir(repository_root).is_err() {
        return refuse("the selected repository working directory is unavailable");
    }
    harness_session_id_availability.select_for_current_process();
    let recovery_command_line = match check_recovery_command_line(&repository_relative_path) {
        Ok(recovery_command_line) => recovery_command_line,
        Err(reason) => return refuse(&reason),
    };
    let check_request = match check_request(&repository_relative_path) {
        Ok(check_request) => check_request,
        Err(reason) => return refuse(&reason),
    };
    let output_envelope = check::execute(check_request, &recovery_command_line);
    render_pre_tool_use_answer(&output_envelope)
}

fn read_request() -> Result<PreToolUseEditAuthorizationRequest, PreToolUsePayloadParseError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| PreToolUsePayloadParseError::InvalidPayload)?;
    let value = serde_json::from_str::<Value>(&input)
        .map_err(|_| PreToolUsePayloadParseError::InvalidPayload)?;
    PreToolUseEditAuthorizationRequest::from_value(&value)
}

/// Place one named edit path in the repository selected by the hook working directory.
fn resolve_named_edit_target(
    edit_path: &Path,
    working_directory: &HookWorkingDirectorySelection,
) -> ResolvedEditTarget {
    if !edit_path.is_absolute() {
        return ResolvedEditTarget::Unresolved {
            reason: "the edit target must be an absolute path".to_owned(),
        };
    }
    let Ok(working_directory) = working_directory.resolve() else {
        return ResolvedEditTarget::Unresolved {
            reason: "the hook working directory is unavailable".to_owned(),
        };
    };
    let Ok(normalized_working_directory) = normalize_absolute_path(&working_directory) else {
        return ResolvedEditTarget::Unresolved {
            reason: "the hook working directory must be an absolute path".to_owned(),
        };
    };
    let worktree_context = match WorktreeContext::discover(&normalized_working_directory) {
        Ok(worktree_context) => worktree_context,
        Err(LedgerError::RepositoryNotFound) => {
            return ResolvedEditTarget::OutsideCoordinationDomain;
        },
        Err(error) => {
            return ResolvedEditTarget::Unresolved {
                reason: format!(
                    "the hook working directory's repository could not be read: {error}"
                ),
            };
        },
    };
    place_in_repository(
        edit_path,
        &CoordinationDomain::around(
            worktree_context.repository_root(),
            &normalized_working_directory,
        ),
    )
}

/// The repository the hook resolved, in the filesystem's namespace and the payload's.
///
/// A payload names paths in whatever namespace the harness runs in, which need not be the
/// canonical one the repository root is discovered in: on macOS a worktree under `/tmp` is
/// discovered as `/private/tmp/...`. Keeping both lets an edit be placed by canonical
/// identity first and still be recognized as in-repository when canonicalization escapes.
struct CoordinationDomain {
    repository_root:                     PathBuf,
    payload_working_directory:           PathBuf,
    working_directory_within_repository: WorkingDirectoryPlacement,
}

/// Whether the hook working directory could be placed inside its own repository root.
enum WorkingDirectoryPlacement {
    /// The working directory is this path relative to the repository root.
    WithinRepository(PathBuf),
    /// The working directory could not be expressed relative to the repository root.
    Unplaceable,
}

impl CoordinationDomain {
    fn around(repository_root: &Path, payload_working_directory: &Path) -> Self {
        let working_directory_within_repository = fs::canonicalize(payload_working_directory)
            .ok()
            .and_then(|canonical_working_directory| {
                canonical_working_directory
                    .strip_prefix(repository_root)
                    .map(Path::to_path_buf)
                    .ok()
            })
            .map_or(
                WorkingDirectoryPlacement::Unplaceable,
                WorkingDirectoryPlacement::WithinRepository,
            );
        Self {
            repository_root: repository_root.to_path_buf(),
            payload_working_directory: payload_working_directory.to_path_buf(),
            working_directory_within_repository,
        }
    }
}

/// Decide whether one payload-named edit path lies inside the resolved coordination domain.
///
/// The path arrives exactly as the payload named it, `..` components included, because only
/// the filesystem can say what a `..` means: `alias/../held.rs` is `held.rs` when `alias` is
/// a real directory and something else entirely when `alias` is a symlink, so collapsing it
/// textually would coordinate a name no write ever reaches.
///
/// Canonical placement runs first so a file reached through a symlink keeps the single
/// coordination identity it has always had. Only when canonicalization lands outside the
/// repository does the payload-namespace comparison run: a symlinked directory inside the
/// worktree that points elsewhere is still an edit inside this worktree, and letting it
/// skip the gate is the regression this ordering exists to prevent.
fn place_in_repository(
    payload_edit_path: &Path,
    coordination_domain: &CoordinationDomain,
) -> ResolvedEditTarget {
    let resolved_edit_path = match canonicalize_through_nearest_existing_ancestor(payload_edit_path)
    {
        Ok(resolved_edit_path) => resolved_edit_path,
        Err(AncestorCanonicalizationError::NoExistingAncestor) => {
            return ResolvedEditTarget::Unresolved {
                reason: "no existing ancestor of the edit target could be resolved".to_owned(),
            };
        },
        Err(AncestorCanonicalizationError::AncestorUnavailable) => {
            return ResolvedEditTarget::Unresolved {
                reason: "the edit target's nearest existing ancestor could not be resolved"
                    .to_owned(),
            };
        },
    };
    resolved_edit_path
        .strip_prefix(&coordination_domain.repository_root)
        .map_or_else(
            |_| place_within_payload_namespace(payload_edit_path, coordination_domain),
            |repository_relative_path| {
                repository_relative_edit_target(
                    &coordination_domain.repository_root,
                    repository_relative_path,
                )
            },
        )
}

/// Place an edit path that escaped the repository when canonicalized, using payload names.
fn place_within_payload_namespace(
    payload_edit_path: &Path,
    coordination_domain: &CoordinationDomain,
) -> ResolvedEditTarget {
    let WorkingDirectoryPlacement::WithinRepository(working_directory_within_repository) =
        &coordination_domain.working_directory_within_repository
    else {
        return ResolvedEditTarget::OutsideCoordinationDomain;
    };
    let Ok(working_directory_relative_path) =
        payload_edit_path.strip_prefix(&coordination_domain.payload_working_directory)
    else {
        return ResolvedEditTarget::OutsideCoordinationDomain;
    };
    let WorktreeRelativeEditName::NamesOneWorktreeFile(working_directory_relative_path) =
        WorktreeRelativeEditName::from_payload_name(working_directory_relative_path)
    else {
        return ResolvedEditTarget::OutsideCoordinationDomain;
    };
    repository_relative_edit_target(
        &coordination_domain.repository_root,
        &working_directory_within_repository.join(working_directory_relative_path),
    )
}

/// Whether the name a payload gave an edit is one a single worktree file answers to.
enum WorktreeRelativeEditName {
    /// Every component names one directory entry, so this name reaches one worktree file.
    NamesOneWorktreeFile(PathBuf),
    /// A `..` component sends the name somewhere the filesystem already placed outside the
    /// repository, so no worktree file answers to it and no scope may be formed from it.
    NamesNoWorktreeFile,
}

impl WorktreeRelativeEditName {
    /// Rebuild a payload-named relative path from the components a worktree file can have.
    ///
    /// The result is assembled only from `Component::Normal`, so it carries no `..` by
    /// construction. That matters twice over: a `..` reaching this point means canonicalization
    /// already placed the write outside the repository, and a scope carrying one would name a
    /// file the write never touches while refusing a peer who really is editing it.
    fn from_payload_name(payload_relative_path: &Path) -> Self {
        let mut worktree_relative_name = PathBuf::new();
        for component in payload_relative_path.components() {
            match component {
                Component::Normal(component) => worktree_relative_name.push(component),
                Component::CurDir => {},
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Self::NamesNoWorktreeFile;
                },
            }
        }
        Self::NamesOneWorktreeFile(worktree_relative_name)
    }
}

/// Convert a repository-relative path into the outcome the check request needs.
fn repository_relative_edit_target(
    repository_root: &Path,
    repository_relative_path: &Path,
) -> ResolvedEditTarget {
    if repository_relative_path.as_os_str().is_empty() {
        return ResolvedEditTarget::OutsideCoordinationDomain;
    }
    let Some(repository_relative_path) = repository_relative_path.to_str() else {
        return ResolvedEditTarget::Unresolved {
            reason: "the repository-relative edit target is not valid Unicode".to_owned(),
        };
    };
    ResolvedEditTarget::WithinRepository {
        repository_root:          repository_root.to_path_buf(),
        repository_relative_path: repository_relative_path.to_owned(),
    }
}

fn check_request(repository_relative_path: &str) -> Result<CheckRequest, String> {
    DeclaredReservationScopeSet::parse(
        vec![PathBuf::from(format!("file:{repository_relative_path}"))],
        ScopeKind::File,
    )
    .map(|declared_scopes| CheckRequest {
        declared_scopes,
        reservation_selection: CheckReservationSelection::SessionMappingOrSingleActive,
    })
    .map_err(|error| error.to_string())
}

fn check_recovery_command_line(
    repository_relative_path: &str,
) -> Result<RecoveryCommandLine, String> {
    RecoveryCommandLine::try_from(vec![
        OsString::from("cargo-berth"),
        OsString::from("check"),
        OsString::from("--json"),
        OsString::from("--"),
        OsString::from(format!("file:{repository_relative_path}")),
    ])
    .map_err(|error| error.to_string())
}

fn refuse(reason: &str) -> ExitCode {
    refuse_hook_request(reason);
    ExitCode::from(BLOCKING_EXIT_CODE)
}

/// The stdout object returned when a `PreToolUse` request remains allowed with a notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreToolUseAllowNotice<'detail> {
    system_message:       &'static str,
    hook_specific_output: PreToolUseAllowNoticeDetail<'detail>,
}

/// The event-specific authorization carried by a `PreToolUse` allow notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreToolUseAllowNoticeDetail<'detail> {
    hook_event_name:            &'static str,
    permission_decision:        &'static str,
    permission_decision_reason: &'detail str,
}

/// Map one engine check answer onto the Claude `PreToolUse` hook protocol.
fn render_pre_tool_use_answer(output_envelope: &OutputEnvelope) -> ExitCode {
    match output_envelope.exit_code() {
        BerthExit::Clear => render_authorized(output_envelope.presentation()),
        BerthExit::LedgerUnreadable => render_fail_open(output_envelope),
        BerthExit::BlockedByOverlap
        | BerthExit::BlockedByOrdering
        | BerthExit::NeedsUserAuthorization
        | BerthExit::UsageError
        | BerthExit::BlockedByContention
        | BerthExit::TerminalViewFailed => render_refusal(output_envelope.presentation()),
    }
}

fn render_authorized(presentation: &EnvelopePresentation) -> ExitCode {
    match presentation {
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_allow_notice(AUTHORIZED_SYSTEM_MESSAGE, &render_blocks(blocks.as_slice()))
        },
        EnvelopePresentation::NothingToShow | EnvelopePresentation::NotProvided => {
            ExitCode::SUCCESS
        },
    }
}

fn render_fail_open(output_envelope: &OutputEnvelope) -> ExitCode {
    match output_envelope.presentation() {
        EnvelopePresentation::NothingToShow => ExitCode::SUCCESS,
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_allow_notice(FAIL_OPEN_SYSTEM_MESSAGE, &render_blocks(blocks.as_slice()))
        },
        EnvelopePresentation::NotProvided => write_allow_notice(
            LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE,
            &output_envelope.render_text(),
        ),
    }
}

fn render_refusal(presentation: &EnvelopePresentation) -> ExitCode {
    match presentation {
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_stderr_line(&render_blocks(blocks.as_slice()));
        },
        EnvelopePresentation::NothingToShow => refuse_hook_request(
            "the engine returned a blocking check answer marked as deliberate silence",
        ),
        EnvelopePresentation::NotProvided => refuse_hook_request(
            "the engine returned a blocking check answer without a presentation",
        ),
    }
    ExitCode::from(BLOCKING_EXIT_CODE)
}

fn write_allow_notice(system_message: &'static str, detail: &str) -> ExitCode {
    let notice = PreToolUseAllowNotice {
        system_message,
        hook_specific_output: PreToolUseAllowNoticeDetail {
            hook_event_name:            HOOK_EVENT_NAME,
            permission_decision:        "allow",
            permission_decision_reason: detail,
        },
    };
    let mut standard_output = std::io::stdout().lock();
    if serde_json::to_writer(&mut standard_output, &notice).is_ok() {
        std::mem::drop(standard_output.write_all(b"\n"));
    }
    ExitCode::SUCCESS
}
