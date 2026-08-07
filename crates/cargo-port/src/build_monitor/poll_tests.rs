//! What the monitor stores from one classified cycle, what it drops as
//! out of scope, and how what it shows ages.

use std::num::NonZeroU64;
use std::time::Instant;

use super::BuildMonitor;
use super::activity::CompileActivityId;
use super::activity::UnattributedScopeEvidence;
use super::classify::BuildClassification;
use super::classify_tests::ClassificationFixture;
use super::execution::CompileMonitorGeneration;
use super::execution::CompletedBuildClassification;
use super::scope::BuildScopeActionability;
use super::scope::BuildScopeKey;
use super::session::LiveOwnedRoot;
use super::session::OwnedRootEvidence;
use super::session::OwnedRootLifecycle;
use super::snapshot::BuildSessionActivity;
use super::snapshot::MonitorDataActionability;
use super::snapshot::MonitorDisplay;
use super::snapshot::MonitorObservation;
use super::snapshot::MonitorSessionOwnership;
use super::snapshot::MonitorSnapshot;
use super::snapshot::MonitorStaleness;
use crate::process_observation::snapshot_builder::ObservedCandidateRole;
use crate::process_observation::snapshot_builder::ObservedProcess;
use crate::process_observation::snapshot_builder::snapshot_of;
use crate::project::AbsolutePath;
use crate::tui::OwnedRunId;

/// A completion stamped with the generation a default monitor still accepts.
fn completed(
    build_scope_key: BuildScopeKey,
    owned_root_evidence: OwnedRootEvidence,
    build_classification: BuildClassification,
) -> CompletedBuildClassification {
    CompletedBuildClassification::new(
        CompileMonitorGeneration::default(),
        build_scope_key,
        owned_root_evidence,
        Box::new(build_classification),
    )
}

fn scope_key_for(path: &std::path::Path) -> BuildScopeKey {
    BuildScopeKey::for_test(AbsolutePath::from(path))
}

fn unrelated_scope_key() -> BuildScopeKey {
    scope_key_for(std::path::Path::new("/nowhere/that/is/indexed"))
}

/// Storing a cycle whose session the scope covers leaves the monitor showing
/// that session under the scope it was classified for.
#[test]
fn a_session_the_scope_covers_is_stored_fresh() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();

    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    let MonitorSnapshot::Fresh(monitor_data) = build_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    assert_eq!(monitor_data.session_rows().len(), 1);
    let monitor_session_row = &monitor_data.session_rows()[0];
    assert_eq!(
        monitor_session_row.session_ownership(),
        MonitorSessionOwnership::External
    );
    assert_eq!(
        monitor_session_row.build_session().build_session_id(),
        monitor_session_row.build_session_id()
    );
    assert_eq!(
        monitor_session_row.build_session_activity(),
        BuildSessionActivity::ActiveWithoutCompiler
    );
    assert_eq!(
        build_monitor.monitor_snapshot().observation(),
        MonitorObservation::Observed(monitor_data.observed_at())
    );
    Ok(())
}

/// The worker classifies every host session and this is the only site that
/// narrows the result, so an external session outside the scope disappears
/// while the Cargo Port-owned run at the same out-of-scope root does not.
#[test]
fn an_out_of_scope_external_session_is_dropped_and_the_owned_run_is_not()
-> Result<(), Box<dyn std::error::Error>> {
    let mut external_fixture = ClassificationFixture::new()?;
    let external_root = external_fixture.cargo_root(&["cargo", "build"]);
    let external_classification = external_fixture.classify(std::slice::from_ref(&external_root));
    let mut build_monitor = BuildMonitor::default();

    build_monitor.record_classification(completed(
        unrelated_scope_key(),
        OwnedRootEvidence::NoLiveRoot,
        external_classification,
    ));

    let MonitorSnapshot::Fresh(monitor_data) = build_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    assert!(monitor_data.session_rows().is_empty());

    let mut owned_fixture = ClassificationFixture::new()?;
    let owned_root = owned_fixture.cargo_root(&["cargo", "build"]);
    let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
    let owned_root_evidence = OwnedRootEvidence::Root(LiveOwnedRoot::new(
        owned_run_id,
        owned_root.identity().clone(),
        std::fs::canonicalize(&owned_fixture.checkout_root)?,
        OwnedRootLifecycle::Live,
    ));
    let owned_classification = owned_fixture.build_classifier.classify_cycle(
        &snapshot_of(&[owned_root]),
        &owned_fixture.cargo_workspace_index,
        &owned_root_evidence,
        Instant::now(),
    );

    build_monitor.record_classification(completed(
        unrelated_scope_key(),
        owned_root_evidence,
        owned_classification,
    ));

    let MonitorSnapshot::Fresh(monitor_data) = build_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    assert_eq!(
        monitor_data.session_rows()[0].session_ownership(),
        MonitorSessionOwnership::Owned(owned_run_id)
    );
    Ok(())
}

/// A cycle that produces no classification moves what is shown one step
/// further from live rather than blanking it, and only the second such cycle
/// leaves nothing to show or act on.
#[test]
fn cycles_without_a_classification_age_what_is_shown() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    build_monitor.record_classification_failure();

    assert!(matches!(
        build_monitor.monitor_snapshot(),
        MonitorSnapshot::Stale(_)
    ));
    assert!(matches!(
        build_monitor.monitor_snapshot().actionability(),
        MonitorDataActionability::NotActionable
    ));

    build_monitor.record_classification_failure();

    assert_eq!(
        *build_monitor.monitor_snapshot(),
        MonitorSnapshot::Unavailable
    );
    Ok(())
}

/// Moving between two rows that cover the same roots keeps the prior rows on
/// screen and keeps them actionable, because a termination re-resolves each
/// identity against the live process snapshot before it signals.
#[test]
fn a_scope_covering_the_same_roots_retains_what_is_shown() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    build_monitor.replace_scope(&BuildScopeActionability::Actionable(scope_key_for(
        &canonical_checkout_root,
    )));

    assert!(matches!(
        build_monitor.monitor_snapshot(),
        MonitorSnapshot::PendingWithRetained(_)
    ));
    assert!(matches!(
        build_monitor.monitor_snapshot().actionability(),
        MonitorDataActionability::Actionable(_)
    ));
    assert!(matches!(
        build_monitor.monitor_snapshot().actionability(),
        MonitorDataActionability::Actionable(monitor_data) if monitor_data.session_rows().len() == 1
    ));
    Ok(())
}

/// A scope covering different roots retains nothing: the prior rows describe
/// checkouts the new scope does not cover.
#[test]
fn a_scope_covering_different_roots_retains_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    build_monitor.replace_scope(&BuildScopeActionability::Actionable(unrelated_scope_key()));

    assert_eq!(*build_monitor.monitor_snapshot(), MonitorSnapshot::Pending);
    Ok(())
}

/// A scope that authorizes no classification shows nothing, whatever was on
/// screen before.
#[test]
fn a_scope_that_authorizes_nothing_shows_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    build_monitor.replace_scope(&BuildScopeActionability::NotActionable);

    assert_eq!(*build_monitor.monitor_snapshot(), MonitorSnapshot::Pending);
    Ok(())
}

/// Switching the monitor off drops everything it was showing and says so, so a
/// reader can tell an off monitor from an enabled one that is still waiting.
#[test]
fn clearing_the_monitor_drops_what_it_was_showing() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    build_monitor.switch_off();

    assert_eq!(*build_monitor.monitor_snapshot(), MonitorSnapshot::Off);
    Ok(())
}

/// The staleness marker survives a scope replacement over the same roots: rows
/// a failed cycle already aged stay stale and stay unable to authorize a
/// termination, because moving the cursor between two rows of one workspace
/// observes nothing about the processes those rows describe.
#[test]
fn staleness_survives_a_scope_replacement_over_the_same_roots()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));
    build_monitor.record_classification_failure();

    build_monitor.replace_scope(&BuildScopeActionability::Actionable(scope_key_for(
        &canonical_checkout_root,
    )));

    assert!(matches!(
        build_monitor.monitor_snapshot(),
        MonitorSnapshot::StaleWithRetained(_)
    ));
    assert!(matches!(
        build_monitor.monitor_snapshot().actionability(),
        MonitorDataActionability::NotActionable
    ));
    assert_eq!(
        drawn_staleness(build_monitor.monitor_snapshot()),
        MonitorStaleness::Stale
    );
    Ok(())
}

/// Every snapshot the monitor can hold names what the pane draws, so the pane
/// never has to decide what an unhandled state should look like.
#[test]
fn every_snapshot_variant_names_what_the_pane_draws() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let build_classification = fixture.classify(std::slice::from_ref(&cargo_root));
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();

    assert!(matches!(
        MonitorSnapshot::Off.monitor_display(),
        MonitorDisplay::SwitchedOff
    ));
    assert!(matches!(
        MonitorSnapshot::Pending.monitor_display(),
        MonitorDisplay::AwaitingFirstCycle
    ));
    assert!(matches!(
        build_monitor.monitor_snapshot().monitor_display(),
        MonitorDisplay::SwitchedOff
    ));

    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));
    assert_eq!(
        drawn_staleness(build_monitor.monitor_snapshot()),
        MonitorStaleness::Live
    );

    build_monitor.replace_scope(&BuildScopeActionability::Actionable(scope_key_for(
        &canonical_checkout_root,
    )));
    assert_eq!(
        drawn_staleness(build_monitor.monitor_snapshot()),
        MonitorStaleness::Live
    );

    build_monitor.record_classification_failure();
    assert_eq!(
        drawn_staleness(build_monitor.monitor_snapshot()),
        MonitorStaleness::Stale
    );

    build_monitor.replace_scope(&BuildScopeActionability::Actionable(scope_key_for(
        &canonical_checkout_root,
    )));
    assert_eq!(
        drawn_staleness(build_monitor.monitor_snapshot()),
        MonitorStaleness::Stale
    );

    build_monitor.record_classification_failure();
    assert!(matches!(
        build_monitor.monitor_snapshot().monitor_display(),
        MonitorDisplay::Unavailable
    ));
    Ok(())
}

/// The staleness marker the pane draws for a snapshot that has rows.
fn drawn_staleness(monitor_snapshot: &MonitorSnapshot) -> MonitorStaleness {
    let MonitorDisplay::Rows {
        monitor_staleness, ..
    } = monitor_snapshot.monitor_display()
    else {
        panic!("this snapshot has rows to draw");
    };
    monitor_staleness
}

/// The scope-level section holds the activities no session claims, and only
/// those: a compiler the classifier confirmed under a session renders as that
/// session's row instead.
///
/// A compiler no session claims is narrowed here by the directory it was
/// observed working in, so a scope covering other checkouts does not inherit
/// another checkout's orphan.
#[test]
fn the_unattributed_section_holds_only_what_no_session_claims()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let confirmed_compiler = ObservedProcess::new(
        202,
        2,
        "rustc lib",
        "/usr/bin/rustc",
        &["rustc", "--crate-name", "demo", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);
    // No validated parent, so nothing attributes it to the open session.
    let orphan_compiler = ObservedProcess::new(
        303,
        3,
        "rustc orphan",
        "/usr/bin/rustc",
        &["rustc", "--crate-name", "orphan", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let build_classification = fixture.classify(&[
        cargo_root.clone(),
        confirmed_compiler.clone(),
        orphan_compiler.clone(),
    ]);
    let canonical_checkout_root = std::fs::canonicalize(&fixture.checkout_root)?;
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        scope_key_for(&canonical_checkout_root),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    let MonitorSnapshot::Fresh(monitor_data) = build_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    assert_eq!(monitor_data.session_rows().len(), 1);
    assert_eq!(
        monitor_data.session_rows()[0].compile_activities().len(),
        1,
        "the confirmed compiler renders as the session's own row"
    );
    let unattributed_activities = monitor_data.unattributed_activities();
    assert_eq!(
        unattributed_activities.len(),
        1,
        "only the compiler no session claims reaches the scope-level section"
    );
    assert_eq!(
        unattributed_activities[0].compile_activity_id(),
        &CompileActivityId::for_test(orphan_compiler.incarnation().clone())
    );
    assert_eq!(
        unattributed_activities[0].scope_evidence(),
        &UnattributedScopeEvidence::WorkingDirectory(AbsolutePath::from(
            canonical_checkout_root.as_path()
        ))
    );

    // The same cycle stored under a scope that covers no root drops the
    // sessions, and drops the orphan with them: it was working inside a
    // checkout this scope does not cover.
    let build_classification = fixture.classify(&[cargo_root, confirmed_compiler, orphan_compiler]);
    let mut out_of_scope_monitor = BuildMonitor::default();
    out_of_scope_monitor.record_classification(completed(
        unrelated_scope_key(),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    let MonitorSnapshot::Fresh(out_of_scope_data) = out_of_scope_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    assert!(out_of_scope_data.session_rows().is_empty());
    assert!(
        out_of_scope_data.unattributed_activities().is_empty(),
        "an orphan working inside another checkout is not this scope's to explain"
    );
    Ok(())
}

/// An orphan whose working directory could not be read is placeable by nothing,
/// so every scope keeps it visible rather than hiding a compiler on the
/// strength of a path the observer never resolved.
#[test]
fn an_unplaceable_unattributed_activity_survives_every_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let unreadable_orphan = ObservedProcess::new(
        404,
        4,
        "rustc orphan",
        "/usr/bin/rustc",
        &["rustc", "--crate-name", "orphan", "src/lib.rs"],
    )
    .with_unreadable_cwd()
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let build_classification = fixture.classify(std::slice::from_ref(&unreadable_orphan));
    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(completed(
        unrelated_scope_key(),
        OwnedRootEvidence::NoLiveRoot,
        build_classification,
    ));

    let MonitorSnapshot::Fresh(monitor_data) = build_monitor.monitor_snapshot() else {
        panic!("a stored cycle is fresh");
    };
    let unattributed_activities = monitor_data.unattributed_activities();
    assert_eq!(unattributed_activities.len(), 1);
    assert_eq!(
        unattributed_activities[0].scope_evidence(),
        &UnattributedScopeEvidence::Unplaceable
    );
    Ok(())
}
