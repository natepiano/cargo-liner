//! Existing integration reservations prepared before the board test clock starts.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::json;

pub(crate) const RESERVATION_IDS_PATH: &str = ".git/integration-parent-reservations.json";

/// An existing reservation covers the integration branch's parent.
pub(crate) const PARENT_RESERVATION: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "integration-parent-reservation",
        build: |root| build(root, ReservationTarget::IntegrationParent),
        ..crate::INITIAL_INTEGRATION_PAIR.repository
    },
    ..crate::INITIAL_INTEGRATION_PAIR
};

/// An existing reservation targets another branch and cannot cover integration.
pub(crate) const UNRELATED_RESERVATION: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "integration-unrelated-reservation",
        build: |root| build(root, ReservationTarget::UnrelatedBranch),
        ..crate::INITIAL_INTEGRATION_PAIR.repository
    },
    ..crate::INITIAL_INTEGRATION_PAIR
};

#[derive(Clone, Copy)]
enum ReservationTarget {
    IntegrationParent,
    UnrelatedBranch,
}

/// The held integration reservation and the ordinary lane's reservation.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct IntegrationParentReservations {
    pub held_id: String,
    pub lane_id: String,
}

fn build(root: &Path, target: ReservationTarget) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["branch", "other-target", "main"]);
    let (initial_target, selected_target) = match target {
        ReservationTarget::IntegrationParent => ("other-target", "main"),
        ReservationTarget::UnrelatedBranch => ("main", "other-target"),
    };
    let held = repo.claim(
        &repo.integration,
        "file:integration.txt",
        FIRST_RUN,
        Some(initial_target),
    );
    repo.assert_success(&held);
    let held_id = json(&held)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("integration reservation ID")
        .to_owned();
    let retargeted = repo.run(
        &repo.integration,
        &["retarget", &held_id, "--target", selected_target, "--json"],
    );
    assert_success(&retargeted);
    let lane = repo.lane("covered-by-ordinary-lane", "integration");
    let claimed = repo.claim(
        &lane,
        "file:lane.txt",
        &uuid::Uuid::now_v7().to_string(),
        None,
    );
    assert_success(&claimed);
    let lane_id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane ID")
        .to_owned();
    if matches!(target, ReservationTarget::IntegrationParent) {
        repo.commit_file(&lane, "lane.txt", "landed\n", "lane work");
    }
    fs::write(
        repo.root().join(RESERVATION_IDS_PATH),
        serde_json::to_vec(&IntegrationParentReservations { held_id, lane_id })
            .expect("prepared reservation ids should encode"),
    )
    .expect("prepared reservation ids should write");
    repo.write_recorded_template(root);
}
