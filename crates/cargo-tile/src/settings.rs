//! cargo-tile's rows in the framework settings overlay, and the
//! stepping that edits them.
//!
//! The framework owns the `[appearance]` steppers, `initial rows`,
//! `fill`, `widen summary`, the Files paths and the Notices section;
//! this module places them and adds cargo-tile's own `fade seconds`
//! stepper and its Capture and Commands rows. Every stepper walks its
//! allowed values on Left/Right/Enter, writes `config.toml`, and swaps
//! the active theme in place. The two Commands lists are typed in:
//! Enter opens the list as comma-separated text, and Enter again writes
//! it and applies it. Every other row reports state and is inert.

use std::io::ErrorKind;
use std::path::PathBuf;

use tui_pane::LIST_SEPARATOR;
use tui_pane::SettingStep;
use tui_pane::SettingTarget;
use tui_pane::SettingsRows;
use tui_pane::apply_settings;
use tui_pane::join_list;
use tui_pane::list_display;
use tui_pane::parse_list;
use tui_pane::step_framework_setting;
use tui_pane::stepped;

use crate::app::App;
use crate::app::CaptureStartupNotice;
use crate::census::SelectedProof;
use crate::config::CargoTile;
use crate::config::Config;
use crate::constants::CAPTURE_ASSOCIATION_AMBIGUOUS;
use crate::constants::CAPTURE_ASSOCIATION_COMPETING;
use crate::constants::CAPTURE_ASSOCIATION_CONFIRMED;
use crate::constants::CAPTURE_ASSOCIATION_SUPPRESSED;
use crate::constants::CAPTURE_ASSOCIATION_UNCONFIRMED;
use crate::constants::CAPTURE_FAILURE_PERMISSION;
use crate::constants::CAPTURE_OWNER_UID;
use crate::constants::CAPTURE_OWNER_UNAVAILABLE;
use crate::constants::CAPTURE_STATUS_ANNOTATION;
use crate::constants::CAPTURE_STATUS_BOOT;
use crate::constants::CAPTURE_STATUS_BOOT_FAILURE;
use crate::constants::CAPTURE_STATUS_CAPTURE;
use crate::constants::CAPTURE_STATUS_EMPTY;
use crate::constants::CAPTURE_STATUS_ENUMERATION;
use crate::constants::CAPTURE_STATUS_ENUMERATION_FAILED;
use crate::constants::CAPTURE_STATUS_IDENTITY;
use crate::constants::CAPTURE_STATUS_IDENTITY_BOOT;
use crate::constants::CAPTURE_STATUS_IDENTITY_RETRY;
use crate::constants::CAPTURE_STATUS_INVALID_REGISTRATION;
use crate::constants::CAPTURE_STATUS_MISSING;
use crate::constants::CAPTURE_STATUS_PARTIAL;
use crate::constants::CAPTURE_STATUS_RETAINED;
use crate::constants::CAPTURE_STATUS_STAGING;
use crate::constants::CAPTURE_STATUS_UNREADABLE_LOG;
use crate::constants::CAPTURE_STATUS_UNREADABLE_REGISTRATION;
use crate::constants::CAPTURE_STATUS_UNSUPPORTED_VERSION;
use crate::constants::CAPTURE_STATUS_UNVERIFIABLE;
use crate::constants::CAPTURE_STATUS_VERSION_RECOVERY;
use crate::constants::CAPTURE_STATUS_YOURS;
use crate::constants::CAPTURE_SUMMARY_SEPARATOR;
use crate::constants::CAPTURE_UNUSED_ROOT_PRECEDENCE;
use crate::constants::CAPTURE_UNUSED_SELECTED_UNCONFIRMED;
use crate::constants::MAX_FADE_SECONDS;
use crate::constants::MIN_FADE_SECONDS;
use crate::constants::REGISTRATION_SEPARATOR;
use crate::progress::capture::CaptureGeneration;
use crate::progress::capture::CaptureKey;
use crate::progress::capture_diagnostic::CaptureDiagnostic;
use crate::progress::capture_diagnostic::PathFailure;
use crate::progress::capture_roots::AccountCaptureDirectory;
use crate::progress::capture_roots::AccountName;
use crate::progress::capture_roots::CaptureCleanup;
use crate::progress::capture_roots::RootReadStatus;
use crate::root_scan::RootOwner;
use crate::root_scan::SharedCaptureDirectory;
use crate::root_scan::SharedDirectoryState;

/// Final selection reaches settings even when no process row can be built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureAssociation {
    /// The displayed process, or the shim when the selection sources no process row.
    pub(crate) pid:       u32,
    /// Retain both successful selection and unresolved competition.
    pub(crate) selection: AssociationSelection,
}

/// Root precedence and verification determine which proof may supply fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AssociationSelection {
    /// One key was selected; other proofs cannot supply metadata.
    Selected {
        /// The exact root, incarnation and generation selected.
        key:    CaptureKey,
        /// Unconfirmed selections permit only log annotation.
        proof:  SelectedProof,
        /// Every confirmed proof left unused has an explicit reason.
        unused: Vec<UnusedCapture>,
    },
    /// Competing publications prevent metadata ownership and ancestor fallback.
    Ambiguous {
        /// Exact identities of the publications that competed in the preferred root.
        candidates: Vec<CaptureKey>,
    },
}

/// A retained proof that the selection forbids using.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnusedCapture {
    /// Preserve publication identity as well as its operator-facing path.
    pub(crate) key:    CaptureKey,
    /// Settings can name the root without reopening it.
    pub(crate) root:   PathBuf,
    /// Explain why this proof supplied neither another row nor borrowed fields.
    pub(crate) reason: UnusedCaptureReason,
}

/// A preferred root remains authoritative whether or not its key was verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnusedCaptureReason {
    /// The selected confirmed publication wins account discovery order.
    RootPrecedence,
    /// The preferred reading is unconfirmed, so another root cannot repair it.
    SelectedUnconfirmed,
}

/// cargo-tile's own editable rows; the framework steps the rest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppSetting {
    /// `tiles.fade_seconds` — steps zero through [`MAX_FADE_SECONDS`].
    FadeSeconds,
    /// One of the `[commands]` lists, typed in.
    Commands(CommandList),
}

/// The `[commands]` lists the settings overlay types into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandList {
    /// `commands.excluded`.
    Excluded,
    /// `commands.hidden_when_idle`.
    HiddenWhenIdle,
}

/// Build the settings rows for the current frame.
pub(crate) fn rows(app: &App) -> SettingsRows<AppSetting> {
    let config = &app.loaded_config.config;
    let mut out = SettingsRows::new();

    out.appearance(&config.appearance);

    out.section("Tiles");
    out.initial_rows(config.tiles.initial_rows);
    out.tile_fill(config.tiles.fill);
    out.widen_summary(config.tiles.widen_summary);
    out.stepper(
        AppSetting::FadeSeconds,
        "fade seconds",
        &config.tiles.fade().as_secs().to_string(),
    );

    out.section("Capture");
    out.value("auto install", config.capture.auto_install.to_string());
    push_capture_directories(&mut out, app);

    out.section("Commands");
    out.text(
        AppSetting::Commands(CommandList::Excluded),
        "excluded",
        list_display(&config.commands.excluded),
    );
    out.text(
        AppSetting::Commands(CommandList::HiddenWhenIdle),
        "hidden when idle",
        list_display(&config.commands.hidden_when_idle),
    );

    out.files::<CargoTile>();

    out.notices(&notices(app));
    out
}

/// Step the selected row's value, then persist and apply the result.
///
/// A read-only row is a no-op, so the keys stay harmless everywhere in
/// the overlay.
pub(crate) fn cycle(app: &mut App, step: SettingStep) {
    let selection = app.framework.settings_pane.viewport().pos();
    match rows(app).target(selection) {
        Some(SettingTarget::Framework(setting)) => {
            step_framework_setting(setting, step, &mut app.loaded_config, &mut app.startup_note);
        },
        Some(SettingTarget::App(AppSetting::FadeSeconds)) => {
            let seconds = fade_choices();
            let current = app.loaded_config.config.tiles.fade().as_secs().to_string();
            app.loaded_config.config.tiles.fade_seconds = stepped(&seconds, &current, step)
                .parse()
                .unwrap_or(MIN_FADE_SECONDS);
            apply_settings(&mut app.loaded_config, &mut app.startup_note);
        },
        Some(
            SettingTarget::App(AppSetting::Commands(_))
            | SettingTarget::AppText(_)
            | SettingTarget::ReadOnly,
        )
        | None => {},
    }
}

/// The selected row's list as the text its editor opens on, when the
/// selection is one of the typed Commands rows.
pub(crate) fn selected_text(app: &App) -> Option<String> {
    let list = selected_command_list(app)?;
    Some(join_list(command_list(&app.loaded_config.config, list)))
}

/// Replace the selected Commands list with what was typed, write
/// `config.toml`, and apply it: the grid reads `hidden_when_idle` every
/// frame, and the census worker picks up `excluded` on its next scan.
pub(crate) fn commit_text(app: &mut App, text: &str) {
    let Some(list) = selected_command_list(app) else {
        return;
    };
    set_command_list(app, list, parse_list(text));
    apply_settings(&mut app.loaded_config, &mut app.startup_note);
}

/// Replace `list` in the config, and in the census worker's copy when
/// it is the one the scan reads.
fn set_command_list(app: &mut App, list: CommandList, entries: Vec<String>) {
    if list == CommandList::Excluded {
        app.excluded_commands.replace(entries.clone());
    }
    *command_list_mut(&mut app.loaded_config.config, list) = entries;
}

/// The Commands list the settings selection is on, if any.
fn selected_command_list(app: &App) -> Option<CommandList> {
    let selection = app.framework.settings_pane.viewport().pos();
    match rows(app).target(selection) {
        Some(SettingTarget::AppText(AppSetting::Commands(list))) => Some(list),
        _ => None,
    }
}

/// The entries `list` holds.
fn command_list(config: &Config, list: CommandList) -> &[String] {
    match list {
        CommandList::Excluded => &config.commands.excluded,
        CommandList::HiddenWhenIdle => &config.commands.hidden_when_idle,
    }
}

/// The entries `list` holds, for replacing.
const fn command_list_mut(config: &mut Config, list: CommandList) -> &mut Vec<String> {
    match list {
        CommandList::Excluded => &mut config.commands.excluded,
        CommandList::HiddenWhenIdle => &mut config.commands.hidden_when_idle,
    }
}

/// The values `tiles.fade_seconds` steps through.
fn fade_choices() -> Vec<String> {
    (MIN_FADE_SECONDS..=MAX_FADE_SECONDS)
        .map(|seconds| seconds.to_string())
        .collect()
}

/// Outstanding startup notices, kept visible after their toasts
/// disappear: the theme note, then capture, then the config error.
fn notices(app: &App) -> Vec<(&'static str, &str)> {
    let mut notices = Vec::new();
    if let Some(note) = &app.startup_note {
        notices.push(("theme", note.as_str()));
    }
    match &app.capture_note {
        CaptureStartupNotice::Quiet => {},
        CaptureStartupNotice::InstallationFailed(note)
        | CaptureStartupNotice::NewerShimKept(note) => notices.push(("capture", note.as_str())),
        CaptureStartupNotice::NewerShimKeptWithFailures { kept, failures } => {
            notices.push(("capture", kept.as_str()));
            notices.push(("capture", failures.as_str()));
        },
    }
    if let Some(error) = &app.loaded_config.error {
        notices.push(("config", error.as_str()));
    }
    notices
}

/// Each effective root adds one selectable value, labelled with its
/// account, whose controls remain inert.
fn push_capture_directories(out: &mut SettingsRows<AppSetting>, app: &App) {
    out.value(
        "shared directory",
        shared_directory_status(&app.shared_directory),
    );
    for status in &app.root_status {
        out.value(&account_label(status), capture_root_status(status));
    }
}

/// The account a root belongs to, by name where the scan resolved one.
pub(crate) fn account_label(status: &AccountCaptureDirectory) -> String {
    match &status.account {
        AccountName::Resolved(name) => name.clone(),
        AccountName::Unavailable => status.root.uid.to_string(),
    }
}

/// Render the sampled parent without filesystem access on the terminal thread.
pub(crate) fn shared_directory_status(directory: &SharedCaptureDirectory) -> String {
    let path = directory.path.display();
    match &directory.state {
        SharedDirectoryState::Missing => {
            format!("{path}{CAPTURE_SUMMARY_SEPARATOR}created by the first captured cargo run")
        },
        SharedDirectoryState::Shared { owner } => format!(
            "{path}{CAPTURE_SUMMARY_SEPARATOR}mode 1777{CAPTURE_SUMMARY_SEPARATOR}{}",
            capture_owner(*owner)
        ),
        SharedDirectoryState::NotShared { mode, owner } => format!(
            "{path}{CAPTURE_SUMMARY_SEPARATOR}mode {mode:04o}{CAPTURE_SUMMARY_SEPARATOR}{}{CAPTURE_SUMMARY_SEPARATOR}other accounts cannot register — run: sudo chmod 1777 /tmp/cargo-tile",
            capture_owner(*owner)
        ),
        SharedDirectoryState::Unavailable(failure) => {
            format!(
                "{path}{CAPTURE_SUMMARY_SEPARATOR}unreadable: {}",
                failure.message
            )
        },
    }
}

/// Render one effective root entirely from observations retained on `App`.
///
/// The first line is the account's summary, and every line after it is
/// something that needs attention. A confirmed association standing
/// alone is the normal case, and adds no line.
pub(crate) fn capture_root_status(status: &AccountCaptureDirectory) -> String {
    if matches!(status.state, RootReadStatus::ForeignOwned { .. }) {
        return capture_read_status(status);
    }
    let mut summary = Vec::new();
    if status.root.cleanup == CaptureCleanup::Here {
        summary.push(CAPTURE_STATUS_YOURS.to_string());
    }
    summary.push(capture_read_status(status));
    // A root that could not be read names its path in the failure.
    if matches!(status.state, RootReadStatus::Readable) {
        summary.push(status.root.path.display().to_string());
    }
    let mut lines = vec![summary.join(CAPTURE_SUMMARY_SEPARATOR)];
    lines.extend(status.diagnostics.iter().map(capture_diagnostic));
    for association in &status.associations {
        lines.extend(association_problems(association));
    }
    lines.join("\n")
}

/// The lines an association adds to its account: none for a confirmed
/// selection with no competing proof, and otherwise the selection and
/// whatever it left unused, or the registrations that compete for it.
fn association_problems(association: &CaptureAssociation) -> Vec<String> {
    match &association.selection {
        AssociationSelection::Selected { key, proof, unused } => {
            if matches!(proof, SelectedProof::Confirmed) && unused.is_empty() {
                return Vec::new();
            }
            let proof = match proof {
                SelectedProof::Confirmed => CAPTURE_ASSOCIATION_CONFIRMED,
                SelectedProof::Unconfirmed => CAPTURE_ASSOCIATION_UNCONFIRMED,
            };
            let mut lines = vec![format!(
                "pid {} via registration {} ({proof})",
                association.pid,
                registration_publication(key),
            )];
            lines.extend(unused.iter().map(|suppressed| {
                let reason = match suppressed.reason {
                    UnusedCaptureReason::RootPrecedence => CAPTURE_UNUSED_ROOT_PRECEDENCE,
                    UnusedCaptureReason::SelectedUnconfirmed => CAPTURE_UNUSED_SELECTED_UNCONFIRMED,
                };
                format!(
                    "{CAPTURE_ASSOCIATION_SUPPRESSED}: pid {} from {} ({}; {reason})",
                    association.pid,
                    suppressed.root.display(),
                    registration_publication(&suppressed.key),
                )
            }));
            lines
        },
        AssociationSelection::Ambiguous { candidates } => {
            let candidates = candidates
                .iter()
                .map(registration_publication)
                .collect::<Vec<_>>()
                .join(LIST_SEPARATOR);
            vec![format!(
                "{CAPTURE_ASSOCIATION_AMBIGUOUS}: pid {}; {CAPTURE_ASSOCIATION_COMPETING}: {candidates}",
                association.pid,
            )]
        },
    }
}

/// The publication basename distinguishes generations sharing a shim pid.
fn registration_publication(key: &CaptureKey) -> String {
    match &key.generation {
        CaptureGeneration::Legacy => key.pid.to_string(),
        CaptureGeneration::Published(generation) => {
            format!("{}{REGISTRATION_SEPARATOR}{generation}", key.pid)
        },
    }
}

/// Keep missing directories, denied access, and readable captures distinct.
fn capture_read_status(status: &AccountCaptureDirectory) -> String {
    match &status.state {
        RootReadStatus::ForeignOwned { owner } => {
            let owner = match owner {
                AccountName::Resolved(name) => name.clone(),
                AccountName::Unavailable => match status.owner {
                    RootOwner::Uid(uid) => uid.to_string(),
                    RootOwner::Unavailable => capture_owner(status.owner),
                },
            };
            format!(
                "{}: owned by {owner}, not by {} — ignored",
                status.root.path.display(),
                account_label(status),
            )
        },
        RootReadStatus::Readable => {
            let partial = format!(
                "{CAPTURE_STATUS_PARTIAL} — {}",
                diagnostic_counts(&status.diagnostics)
            );
            // A partial read with nothing confirmed does not know there
            // are no active captures, so it does not say so.
            match (status.confirmed, status.diagnostics.is_empty()) {
                (0, true) => CAPTURE_STATUS_EMPTY.to_string(),
                (0, false) => partial,
                (confirmed, true) => counted(confirmed, CAPTURE_STATUS_CAPTURE),
                (confirmed, false) => format!(
                    "{}{CAPTURE_SUMMARY_SEPARATOR}{partial}",
                    counted(confirmed, CAPTURE_STATUS_CAPTURE)
                ),
            }
        },
        RootReadStatus::Unavailable(failure) if failure.failure.kind == ErrorKind::NotFound => {
            format!("{CAPTURE_STATUS_MISSING}: {}", path_failure(failure))
        },
        RootReadStatus::Unavailable(failure) => path_failure(failure),
    }
}

/// Aggregate artifact kinds without promoting any of them to confirmed captures.
fn diagnostic_counts(diagnostics: &[CaptureDiagnostic]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for diagnostic in diagnostics {
        let label = diagnostic_label(diagnostic);
        if let Some((_, count)) = counts.iter_mut().find(|(known, _)| *known == label) {
            *count += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .into_iter()
        .map(|(label, count)| counted(count, label))
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}

/// All count labels use regular plurals; one capture remains singular.
fn counted(count: usize, label: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {label}{plural}")
}

/// Each retained observation has its own operator-facing count category.
const fn diagnostic_label(diagnostic: &CaptureDiagnostic) -> &'static str {
    match diagnostic {
        CaptureDiagnostic::EnumerationIncomplete(_) => CAPTURE_STATUS_ENUMERATION,
        CaptureDiagnostic::EnumerationFailed(_) => CAPTURE_STATUS_ENUMERATION_FAILED,
        CaptureDiagnostic::RegistrationUnreadable(_) => CAPTURE_STATUS_UNREADABLE_REGISTRATION,
        CaptureDiagnostic::RegistrationInvalid(_) => CAPTURE_STATUS_INVALID_REGISTRATION,
        CaptureDiagnostic::UnsupportedRegistrationVersion { .. } => {
            CAPTURE_STATUS_UNSUPPORTED_VERSION
        },
        CaptureDiagnostic::AnnotationOnly(_) => CAPTURE_STATUS_ANNOTATION,
        CaptureDiagnostic::Unverifiable(_) => CAPTURE_STATUS_UNVERIFIABLE,
        CaptureDiagnostic::IdentityUnknown(_) | CaptureDiagnostic::IdentityBlockedByBoot(_) => {
            CAPTURE_STATUS_IDENTITY
        },
        CaptureDiagnostic::Staging(_) => CAPTURE_STATUS_STAGING,
        CaptureDiagnostic::LogUnreadable(_) => CAPTURE_STATUS_UNREADABLE_LOG,
        CaptureDiagnostic::BootUnavailable(_) => CAPTURE_STATUS_BOOT_FAILURE,
    }
}

/// Path-qualified details explain retention without promising eventual deletion.
fn capture_diagnostic(diagnostic: &CaptureDiagnostic) -> String {
    let label = diagnostic_label(diagnostic);
    match diagnostic {
        CaptureDiagnostic::UnsupportedRegistrationVersion {
            path,
            encountered,
            supported,
        } => format!(
            "{label}: {} (encountered v{encountered}; this reader supports v{supported}; {CAPTURE_STATUS_VERSION_RECOVERY})",
            path.display()
        ),
        CaptureDiagnostic::EnumerationIncomplete(path) | CaptureDiagnostic::Staging(path) => {
            format!("{label}: {}", path.display())
        },
        CaptureDiagnostic::RegistrationInvalid(path)
        | CaptureDiagnostic::AnnotationOnly(path)
        | CaptureDiagnostic::Unverifiable(path) => {
            format!("{label}: {} ({CAPTURE_STATUS_RETAINED})", path.display())
        },
        CaptureDiagnostic::IdentityUnknown(path) => {
            format!(
                "{label}: {} ({CAPTURE_STATUS_IDENTITY_RETRY})",
                path.display()
            )
        },
        CaptureDiagnostic::IdentityBlockedByBoot(path) => {
            format!(
                "{label}: {} ({CAPTURE_STATUS_IDENTITY_BOOT})",
                path.display()
            )
        },
        CaptureDiagnostic::EnumerationFailed(failure)
        | CaptureDiagnostic::RegistrationUnreadable(failure)
        | CaptureDiagnostic::LogUnreadable(failure) => {
            format!("{label}: {}", path_failure(failure))
        },
        CaptureDiagnostic::BootUnavailable(failure) => {
            format!("{CAPTURE_STATUS_BOOT}: {}", path_failure(failure))
        },
    }
}

/// Account identity comes from the scanned descriptor, never an account lookup.
fn capture_owner(owner: RootOwner) -> String {
    match owner {
        RootOwner::Uid(uid) => format!("{CAPTURE_OWNER_UID} {uid}"),
        RootOwner::Unavailable => CAPTURE_OWNER_UNAVAILABLE.to_string(),
    }
}

/// Both the failed artifact and the original diagnostic survive rendering.
fn path_failure(failure: &PathFailure) -> String {
    match failure.failure.kind {
        ErrorKind::PermissionDenied => format!(
            "{CAPTURE_FAILURE_PERMISSION}: {} ({})",
            failure.path.display(),
            failure.failure.message,
        ),
        _ => format!("{}: {}", failure.path.display(), failure.failure.message),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::ErrorKind;

    use tui_pane::SettingTarget;
    use tui_pane::SettingsRow;
    use tui_pane::SettingsRowIdentity;
    use tui_pane::parse_list;

    use super::AssociationSelection;
    use super::CaptureAssociation;
    use super::CommandList;
    use super::UnusedCaptureReason;
    use super::account_label;
    use super::rows;
    use super::selected_text;
    use super::set_command_list;
    use crate::app::App;
    use crate::app::CaptureStartupNotice;
    use crate::birth_stamp::IdentityEvidence;
    use crate::census::SelectedProof;
    use crate::constants::CAPTURE_ASSOCIATION_AMBIGUOUS;
    use crate::constants::CAPTURE_ASSOCIATION_CONFIRMED;
    use crate::constants::CAPTURE_ASSOCIATION_UNCONFIRMED;
    use crate::constants::CAPTURE_STATUS_IDENTITY_BOOT;
    use crate::constants::CAPTURE_UNUSED_ROOT_PRECEDENCE;
    use crate::constants::CAPTURE_UNUSED_SELECTED_UNCONFIRMED;
    use crate::constants::SUPPORTED_REGISTRATION_VERSION;
    use crate::progress::capture::CaptureGeneration;
    use crate::progress::capture::CaptureKey;
    use crate::progress::capture::CaptureRootIndex;
    use crate::progress::capture_diagnostic::CaptureDiagnostic;
    use crate::progress::capture_diagnostic::CaptureFailure;
    use crate::progress::capture_diagnostic::PathFailure;
    use crate::progress::capture_roots::AccountCaptureDirectory;
    use crate::progress::capture_roots::AccountName;
    use crate::progress::capture_roots::CaptureCleanup;
    use crate::progress::capture_roots::CaptureRoot;
    use crate::progress::capture_roots::RootReadStatus;
    use crate::root_scan::RootOwner;

    /// Construct retained evidence without resolving or accessing a fixture path.
    fn observed_root() -> AccountCaptureDirectory {
        AccountCaptureDirectory {
            root:    CaptureRoot {
                path:    "/retained/captures".into(),
                uid:     1000,
                cleanup: CaptureCleanup::Here,
            },
            owner:   RootOwner::Uid(1000),
            account: AccountName::Unavailable,

            state:        RootReadStatus::Readable,
            confirmed:    0,
            diagnostics:  Vec::new(),
            associations: Vec::new(),
        }
    }

    /// Give settings a retained publication identity without scanning a process.
    fn association_key(pid: u32, root: usize, generation: &str) -> CaptureKey {
        let directory = tempfile::tempdir().expect("capture root");
        let scan = crate::root_scan::RootScan::open(
            directory.path(),
            &mut crate::root_scan::RootHistory::default(),
        )
        .expect("open capture root");
        CaptureKey {
            root: CaptureRootIndex(root),
            pid,
            incarnation: scan.incarnation(),
            generation: CaptureGeneration::Published(generation.into()),
            birth: IdentityEvidence::Unavailable,
        }
    }

    /// Preserve the exact failed path and diagnostic without creating an I/O error.
    fn failure(path: &str, kind: ErrorKind) -> PathFailure {
        PathFailure {
            path:    path.into(),
            failure: CaptureFailure {
                kind,
                message: "retained scan error".into(),
            },
        }
    }

    /// Exercise the public row builder, including its inert selection mapping.
    fn root_row(status: AccountCaptureDirectory) -> SettingsRow {
        let mut app = App::new_for_test().expect("quiet settings app");
        let label = account_label(&status);
        app.root_status.push(status);
        let settings = rows(&app);
        let root = settings
            .rows()
            .iter()
            .find(|row| row.label == label)
            .expect("retained root row");
        assert_eq!(root.kind, SettingsRow::value(0, "", "").kind);
        let SettingsRowIdentity::Selectable(payload) = root.identity else {
            panic!("account row must carry a selectable identity");
        };
        assert_eq!(
            settings.target(payload.get()),
            Some(SettingTarget::ReadOnly)
        );
        root.clone()
    }

    #[test]
    fn readable_empty_root_displays_owner_and_path_without_cleanup() {
        assert_eq!(
            root_row(observed_root()).value,
            "yours · no active captures · /retained/captures",
        );
    }

    #[test]
    fn confirmed_captures_have_singular_and_plural_counts() {
        for (confirmed, wording) in [(1, "yours · 1 capture ·"), (2, "yours · 2 captures ·")] {
            let mut status = observed_root();
            status.confirmed = confirmed;
            assert!(root_row(status).value.contains(wording));
        }
    }

    #[test]
    fn foreign_owner_is_read_only_without_a_permission_repair_instruction() {
        let mut status = observed_root();
        status.owner = RootOwner::Uid(2000);
        status.root.uid = 2000;
        status.root.cleanup = CaptureCleanup::AccountNextRun;
        let value = root_row(status).value;
        assert_eq!(value, "no active captures · /retained/captures");
        assert!(!value.contains("restart"));
        assert!(!value.contains("writable"));
    }

    #[test]
    fn missing_directory_is_distinct_from_failed_access_and_empty() {
        let mut status = observed_root();
        status.owner = RootOwner::Unavailable;
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::NotFound));
        let value = root_row(status).value;
        assert!(value.starts_with("yours · missing directory: /retained/captures"));
        assert!(!value.contains("no active captures"));
        assert!(!value.contains("restart"));
    }

    #[test]
    fn root_permission_failure_preserves_its_path_and_observed_message() {
        let mut status = observed_root();
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::PermissionDenied));
        assert!(
            root_row(status)
                .value
                .contains("permission denied: /retained/captures (retained scan error)")
        );
    }

    #[test]
    fn failed_directory_enumeration_names_state_pids_in_the_row() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::EnumerationFailed(failure(
                "/retained/captures/state/pids",
                ErrorKind::PermissionDenied,
            )));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 failed directory listing"));
        assert!(
            value
                .contains("permission denied: /retained/captures/state/pids (retained scan error)")
        );
        assert!(!value.contains("no active captures"));
    }

    #[test]
    fn bounded_enumeration_is_distinct_from_a_failed_enumeration() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::EnumerationIncomplete(
                "/retained/captures/state/pids".into(),
            ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 short directory listing"));
        assert!(value.contains("short directory listing: /retained/captures/state/pids"));
        assert!(!value.contains("failed directory listing"));
    }

    #[test]
    fn unreadable_registration_is_distinct_from_an_empty_root() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::RegistrationUnreadable(failure(
                "/retained/captures/state/pids/42-uuid",
                ErrorKind::PermissionDenied,
            )));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unreadable registration"));
        assert!(value.contains("permission denied: /retained/captures/state/pids/42-uuid"));
        assert!(!value.contains("no active captures"));
    }

    #[test]
    fn invalid_registration_is_visible_with_its_artifact_path() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::RegistrationInvalid(
                "/retained/captures/state/pids/42-uuid".into(),
            ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 invalid registration"));
        assert!(value.contains("invalid registration: /retained/captures/state/pids/42-uuid"));
    }

    #[test]
    fn unsupported_version_names_both_versions_and_reader_recovery() {
        let mut status = observed_root();
        let encountered = SUPPORTED_REGISTRATION_VERSION + 1;
        status
            .diagnostics
            .push(CaptureDiagnostic::UnsupportedRegistrationVersion {
                path: "/retained/captures/state/pids/42-uuid".into(),
                encountered,
                supported: SUPPORTED_REGISTRATION_VERSION,
            });
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unsupported registration version"));
        assert!(value.contains("/retained/captures/state/pids/42-uuid"));
        assert!(value.contains(&format!("encountered v{encountered}")));
        assert!(value.contains(&format!(
            "this reader supports v{SUPPORTED_REGISTRATION_VERSION}"
        )));
        assert!(value.contains("registration and log retained"));
        assert!(value.contains("upgrade and restart the reader"));
        assert!(!value.contains("invalid registration"));
    }

    #[test]
    fn kept_newer_shim_and_installation_failure_remain_separate_settings_rows() {
        let kept = "stable: newer shim kept; upgrade this older reader";
        let failures = "nightly: not installed: permission denied";
        let mut app = App::new_for_test().expect("quiet settings app");
        for (notice, expected) in [
            (CaptureStartupNotice::Quiet, vec![]),
            (
                CaptureStartupNotice::InstallationFailed(failures.into()),
                vec![failures],
            ),
            (CaptureStartupNotice::NewerShimKept(kept.into()), vec![kept]),
            (
                CaptureStartupNotice::NewerShimKeptWithFailures {
                    kept:     kept.into(),
                    failures: failures.into(),
                },
                vec![kept, failures],
            ),
        ] {
            app.capture_note = notice;
            for _ in 0..2 {
                let settings = rows(&app);
                let actual: Vec<_> = settings
                    .rows()
                    .iter()
                    .filter(|row| row.label == "capture")
                    .map(|row| row.value.as_str())
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn mixed_root_counts_only_confirmed_captures_and_names_each_other_artifact() {
        let mut status = observed_root();
        status.confirmed = 1;
        status.diagnostics = vec![
            CaptureDiagnostic::LogUnreadable(failure(
                "/retained/captures/run-42-uuid.log",
                ErrorKind::PermissionDenied,
            )),
            CaptureDiagnostic::AnnotationOnly("/retained/captures/state/pids/43".into()),
            CaptureDiagnostic::Staging("/retained/captures/state/pids/44-uuid.tmp".into()),
        ];
        let value = root_row(status).value;
        assert!(value.contains(
            "1 capture · partial — 1 unreadable log, 1 annotation-only record, 1 staging file"
        ));
        assert!(value.contains("permission denied: /retained/captures/run-42-uuid.log"));
        assert!(value.contains("annotation-only record: /retained/captures/state/pids/43"));
        assert!(value.contains("staging file: /retained/captures/state/pids/44-uuid.tmp"));
        assert!(!value.contains("4 captures"));
    }

    #[test]
    fn repeated_unreadable_logs_are_counted_together_and_keep_both_paths() {
        let mut status = observed_root();
        for path in [
            "/retained/captures/run-42.log",
            "/retained/captures/run-43.log",
        ] {
            status
                .diagnostics
                .push(CaptureDiagnostic::LogUnreadable(failure(
                    path,
                    ErrorKind::Other,
                )));
        }
        let value = root_row(status).value;
        assert!(value.contains("partial — 2 unreadable logs"));
        assert!(value.contains("/retained/captures/run-42.log: retained scan error"));
        assert!(value.contains("/retained/captures/run-43.log: retained scan error"));
    }

    #[test]
    fn boot_failure_names_restart_and_differs_from_identity_unknown_this_scan() {
        let mut cached = observed_root();
        cached
            .diagnostics
            .push(CaptureDiagnostic::BootUnavailable(failure(
                "/kernel/boot-id",
                ErrorKind::PermissionDenied,
            )));
        let cached = root_row(cached).value;
        assert!(cached.contains("partial — 1 cached boot failure"));
        assert!(cached.contains("boot identity unavailable — verification disabled for this session; restart to retry boot read"));
        assert!(cached.contains("/kernel/boot-id"));
        let mut retryable = observed_root();
        retryable
            .diagnostics
            .push(CaptureDiagnostic::IdentityUnknown(
                "/retained/captures/state/pids/42-uuid".into(),
            ));
        let retryable = root_row(retryable).value;
        assert!(retryable.contains("partial — 1 unconfirmed registration"));
        assert!(retryable.contains(
            "identity unknown this scan — retained; identity is checked again next scan"
        ));
        assert!(!retryable.contains("restart"));
        assert_ne!(cached, retryable);
    }

    #[test]
    fn boot_blocked_identity_keeps_retention_without_promising_a_next_scan_retry() {
        let mut status = observed_root();
        status.diagnostics = vec![
            CaptureDiagnostic::IdentityBlockedByBoot(
                "/retained/captures/state/pids/42-uuid".into(),
            ),
            CaptureDiagnostic::BootUnavailable(failure(
                "/kernel/boot-id",
                ErrorKind::PermissionDenied,
            )),
        ];
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unconfirmed registration, 1 cached boot failure"));
        assert!(value.contains(&format!(
            "unconfirmed registration: /retained/captures/state/pids/42-uuid ({CAPTURE_STATUS_IDENTITY_BOOT})",
        )));
        assert!(!value.contains("cleanup"));
        assert!(value.contains("boot identity unavailable — verification disabled for this session; restart to retry boot read: permission denied: /kernel/boot-id"));
        assert!(!value.contains("next scan"));
        assert!(!value.contains("1 capture"));
    }

    #[test]
    fn permanently_unverifiable_artifact_does_not_promise_eventual_removal() {
        let mut status = observed_root();
        status.diagnostics.push(CaptureDiagnostic::Unverifiable(
            "/retained/captures/state/pids/42-uuid".into(),
        ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unverifiable artifact"));
        assert!(value.contains("retained — identity cannot be established"));
        assert!(!value.contains("next scan"));
        assert!(!value.contains("eventual"));
        assert!(!value.contains("will remove"));
    }

    #[test]
    fn association_names_its_supplier_and_any_unused_competing_proof() {
        let mut status = observed_root();
        status.associations = vec![CaptureAssociation {
            pid:       42,
            selection: AssociationSelection::Selected {
                key:    association_key(40, 0, "selected"),
                proof:  SelectedProof::Unconfirmed,
                unused: vec![crate::settings::UnusedCapture {
                    key:    association_key(40, 1, "unused"),
                    root:   "/retained/other".into(),
                    reason: UnusedCaptureReason::SelectedUnconfirmed,
                }],
            },
        }];
        let value = root_row(status).value;
        assert!(value.contains("pid 42 via registration 40.selected"));
        assert!(value.contains("another root's proof went unused: pid 42 from /retained/other"));
    }

    /// The retained association alone explains selection for cargo rows and shim fallbacks.
    #[test]
    fn selected_proofs_and_suppression_reasons_render_for_both_row_sources() {
        for pid in [40, 42] {
            for (proof, reason, expected) in [
                (
                    SelectedProof::Confirmed,
                    UnusedCaptureReason::RootPrecedence,
                    CAPTURE_UNUSED_ROOT_PRECEDENCE,
                ),
                (
                    SelectedProof::Unconfirmed,
                    UnusedCaptureReason::SelectedUnconfirmed,
                    CAPTURE_UNUSED_SELECTED_UNCONFIRMED,
                ),
            ] {
                let mut status = observed_root();
                status.associations.push(CaptureAssociation {
                    pid,
                    selection: AssociationSelection::Selected {
                        key: association_key(40, 0, "selected"),
                        proof,
                        unused: vec![crate::settings::UnusedCapture {
                            key: association_key(40, 1, "unused"),
                            root: "/retained/other".into(),
                            reason,
                        }],
                    },
                });
                let value = root_row(status).value;
                assert!(
                    value.contains(&format!("pid {pid} via registration 40.selected")),
                    "{value}"
                );
                assert!(value.contains("40.selected"), "{value}");
                assert!(value.contains("40.unused"), "{value}");
                assert!(value.contains(expected), "{value}");
                let expected_proof = match proof {
                    SelectedProof::Confirmed => CAPTURE_ASSOCIATION_CONFIRMED,
                    SelectedProof::Unconfirmed => CAPTURE_ASSOCIATION_UNCONFIRMED,
                };
                assert!(value.contains(expected_proof), "{value}");
            }
        }
    }

    /// The settings app has no process rows; both association shapes still retain competition.
    #[test]
    fn ambiguous_generations_render_with_and_without_a_represented_process() {
        for pid in [40, 42] {
            let first = association_key(40, 0, "first");
            let mut second = first.clone();
            second.generation = CaptureGeneration::Published("second".into());
            let mut status = observed_root();
            status.associations.push(CaptureAssociation {
                pid,
                selection: AssociationSelection::Ambiguous {
                    candidates: vec![first.clone(), second],
                },
            });
            let ambiguous = root_row(status.clone()).value;
            assert!(
                ambiguous.contains(CAPTURE_ASSOCIATION_AMBIGUOUS),
                "{ambiguous}"
            );
            assert!(
                ambiguous.contains(&format!("{CAPTURE_ASSOCIATION_AMBIGUOUS}: pid {pid};")),
                "{ambiguous}"
            );
            assert!(ambiguous.contains("40.first"), "{ambiguous}");
            assert!(ambiguous.contains("40.second"), "{ambiguous}");
            status.associations[0].selection = AssociationSelection::Selected {
                key:    first,
                proof:  SelectedProof::Confirmed,
                unused: Vec::new(),
            };
            let recovered = root_row(status).value;
            assert_eq!(
                recovered, "yours · no active captures · /retained/captures",
                "a confirmed selection standing alone adds no line"
            );
            assert_ne!(ambiguous, recovered);
        }
    }

    /// A verified row source does not turn its unreadable log into an active capture.
    #[test]
    fn selected_proof_preserves_unreadable_log_diagnostic_and_active_count() {
        let mut status = observed_root();
        status.confirmed = 1;
        status
            .diagnostics
            .push(CaptureDiagnostic::LogUnreadable(failure(
                "/retained/captures/run-40-selected.log",
                ErrorKind::PermissionDenied,
            )));
        status.associations.push(CaptureAssociation {
            pid:       40,
            selection: AssociationSelection::Selected {
                key:    association_key(40, 0, "selected"),
                proof:  SelectedProof::Confirmed,
                unused: Vec::new(),
            },
        });
        let value = root_row(status).value;
        assert!(value.contains("1 capture"), "{value}");
        assert!(value.contains("1 unreadable log"), "{value}");
        assert!(
            value.contains("permission denied: /retained/captures/run-40-selected.log"),
            "{value}"
        );
        assert!(!value.contains(CAPTURE_ASSOCIATION_CONFIRMED), "{value}");
    }

    #[test]
    fn rows_use_retained_observations_without_filesystem_access() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let mut status = observed_root();
        // A NUL cannot occur in a real Unix pathname. Reopening this root could
        // never produce the retained readable status, owner, or confirmed count.
        status.root.path = "/retained/\0/captures".into();
        status.confirmed = 2;
        app.root_status.push(status);
        let first = rows(&app);
        let second = rows(&app);
        assert_eq!(first.rows(), second.rows());
        let root = first
            .rows()
            .iter()
            .find(|row| row.label == "1000")
            .expect("root row");
        assert!(root.value.starts_with("yours"));
        assert!(root.value.contains("2 captures"));
        assert!(root.value.contains("/retained/\0/captures"));
    }

    #[test]
    fn retained_access_recovery_changes_the_row_without_process_groups() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let mut status = observed_root();
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::NotFound));
        app.root_status.push(status);
        let missing = rows(&app).rows().to_vec();
        app.root_status[0].state = RootReadStatus::Readable;
        let readable = rows(&app).rows().to_vec();
        assert_ne!(missing, readable);
        app.root_status[0]
            .diagnostics
            .push(CaptureDiagnostic::LogUnreadable(failure(
                "/retained/captures/run-42.log",
                ErrorKind::PermissionDenied,
            )));
        let unreadable_log = rows(&app).rows().to_vec();
        assert_ne!(readable, unreadable_log);
        app.root_status[0].diagnostics.clear();
        assert_eq!(rows(&app).rows(), readable);
    }

    /// Select the row `label` names, as a click or the arrows would.
    fn select(app: &mut App, label: &str) {
        let settings = rows(app);
        let selection = settings
            .rows()
            .iter()
            .find_map(|row| match row.identity {
                SettingsRowIdentity::Selectable(payload) if row.label == label => {
                    Some(payload.get())
                },
                _ => None,
            })
            .expect("selectable settings row");
        app.framework.settings_pane.select_row(selection);
    }

    /// The editor opens on the entries themselves: an empty list reads
    /// `none` in the row but opens as nothing to type over.
    #[test]
    fn only_the_command_lists_open_an_editor_on_their_bare_entries() {
        let mut app = App::new_for_test().expect("quiet settings app");
        app.loaded_config.config.commands.hidden_when_idle = vec!["port".into(), "handler".into()];
        app.loaded_config.config.commands.excluded.clear();
        select(&mut app, "hidden when idle");
        assert_eq!(selected_text(&app).as_deref(), Some("port, handler"));
        select(&mut app, "excluded");
        assert_eq!(selected_text(&app).as_deref(), Some(""));
        select(&mut app, "fade seconds");
        assert_eq!(selected_text(&app), None);
        select(&mut app, "auto install");
        assert_eq!(selected_text(&app), None);
    }

    /// `hidden_when_idle` is read from the config every frame, while the
    /// census worker holds its own handle on `excluded`.
    #[test]
    fn a_typed_list_replaces_the_config_and_reaches_the_census_worker() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let worker = app.excluded_commands.clone();
        set_command_list(
            &mut app,
            CommandList::HiddenWhenIdle,
            parse_list("port handler"),
        );
        assert_eq!(
            app.loaded_config.config.commands.hidden_when_idle,
            ["port", "handler"]
        );
        assert_eq!(
            worker.snapshot(),
            app.loaded_config.config.commands.excluded
        );
        set_command_list(&mut app, CommandList::Excluded, parse_list("berth, fmt"));
        assert_eq!(app.loaded_config.config.commands.excluded, ["berth", "fmt"]);
        assert_eq!(worker.snapshot(), ["berth", "fmt"]);
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod layout_tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tui_pane::AppIdentity;
    use tui_pane::GlobalAction;
    use tui_pane::SECTION_ITEM_INDENT;
    use tui_pane::SettingsRow;
    use tui_pane::SettingsRowIdentity;

    use super::rows;
    use super::shared_directory_status;
    use crate::app::App;
    use crate::app::CaptureStartupNotice;
    use crate::config::CargoTile;
    use crate::render;

    /// A resolved path as the Files rows print it.
    fn shown(path: Option<PathBuf>) -> String {
        path.map_or_else(
            || "unavailable".to_string(),
            |path| path.display().to_string(),
        )
    }

    /// Each row as `[section]` or `payload kind label = value`, with the
    /// machine-specific Files paths replaced by placeholders.
    fn layout(app: &App) -> Vec<String> {
        let paths = [
            (
                shared_directory_status(&app.shared_directory),
                "<shared directory>",
            ),
            (shown(CargoTile::config_path()), "<config path>"),
            (shown(CargoTile::themes_dir()), "<themes dir>"),
            (shown(CargoTile::keymap_path()), "<keymap path>"),
        ];
        rows(app)
            .rows()
            .iter()
            .map(|row| {
                let value = paths
                    .iter()
                    .find(|(path, _)| *path == row.value)
                    .map_or(row.value.as_str(), |(_, placeholder)| placeholder);
                match row.identity {
                    SettingsRowIdentity::Decoration => format!("[{}]", row.label),
                    SettingsRowIdentity::Selectable(payload) => {
                        format!("{} {:?} {} = {value}", payload.get(), row.kind, row.label)
                    },
                }
            })
            .collect()
    }

    /// Rows shared by both layouts: everything above Notices.
    const BODY: [&str; 19] = [
        "[Appearance]",
        "0 Stepper mode = auto",
        "1 Stepper light theme = Default Light",
        "2 Stepper dark theme = Default Dark",
        "[Tiles]",
        "3 Stepper initial rows = 4",
        "4 Stepper fill = redistribute",
        "5 Stepper widen summary = false",
        "6 Stepper fade seconds = 3",
        "[Capture]",
        "7 Value auto install = true",
        "8 Value shared directory = <shared directory>",
        "[Commands]",
        "9 Value excluded = berth",
        "10 Value hidden when idle = port, handler",
        "[Files]",
        "11 Value config = <config path>",
        "12 Value themes = <themes dir>",
        "13 Value keymap = <keymap path>",
    ];

    /// Order, section headers, labels, values, row kinds and selectable
    /// payloads, captured from the build before the settings overlay
    /// moved into `tui_pane`.
    #[test]
    fn settings_rows_keep_their_layout() {
        let mut app = App::new_for_test().expect("quiet settings app");
        assert_eq!(layout(&app), BODY);

        app.startup_note = Some("theme note".to_string());
        app.capture_note = CaptureStartupNotice::NewerShimKeptWithFailures {
            kept:     "kept note".to_string(),
            failures: "failure note".to_string(),
        };
        app.loaded_config.error = Some("config error".to_string());
        let mut expected = BODY.to_vec();
        expected.extend([
            "[Notices]",
            "14 Value theme = theme note",
            "15 Value capture = kept note",
            "16 Value capture = failure note",
            "17 Value config = config error",
        ]);
        assert_eq!(layout(&app), expected);
    }

    /// The popup is as wide as its widest row plus the border, never
    /// narrower than 64 cells, measured from the drawn frame.
    #[test]
    fn settings_popup_fits_its_widest_row() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let built = rows(&app).rows().to_vec();
        let section = SettingsRow::section("").kind;
        let stepper = SettingsRow::stepper(0, "", "").kind;
        let label = built
            .iter()
            .filter(|row| row.kind != section)
            .map(|row| row.label.chars().count())
            .max()
            .unwrap_or(0);
        let value = built
            .iter()
            .filter(|row| row.kind != section)
            .map(|row| {
                let decoration = if row.kind == stepper { 4 } else { 0 };
                row.value.chars().count() + decoration
            })
            .max()
            .unwrap_or(0);
        let widest = SECTION_ITEM_INDENT.chars().count() + 2 + label + 2 + value;
        let expected = (widest + 2).max(64);

        let keymap = Rc::clone(&app.keymap);
        keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
        let mut terminal =
            Terminal::new(TestBackend::new(400, 60)).expect("create settings terminal");
        terminal
            .draw(|frame| render::draw(frame, &mut app, &keymap))
            .expect("draw settings");
        let buffer = terminal.backend().buffer();
        let top = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .find(|line| line.contains(" Settings "))
            .expect("settings popup title");
        let left = top
            .chars()
            .position(|cell| cell == '┌')
            .expect("left border");
        let right = top
            .chars()
            .position(|cell| cell == '┐')
            .expect("right border");
        assert_eq!(right - left + 1, expected);
    }
}
