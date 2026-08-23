use std::collections::BTreeMap;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;
use tempfile::TempDir;

use crate::project::AbsolutePath;
use crate::project::AcceptedCargoMetadataRevision;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalPathResolution;
use crate::project::CanonicalTargetDirectory;
use crate::project::CanonicalWorkspaceRoot;
use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceIndexRevision;
use crate::project::CargoWorkspaceIndexRevisionState;
use crate::project::CargoWorkspaceView;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::ProjectListRevision;
use crate::project::PublishPolicy;
use crate::project::TargetRecord;
use crate::project::VisibleTargetWorkspaceOwnership;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct WorkspaceIndexApiFixture {
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    cargo_workspace_index:            CargoWorkspaceIndex,
    cargo_workspace_root:             PathBuf,
    checkout_root:                    PathBuf,
    project_list_revision:            ProjectListRevision,
    resolved_member_root:             PathBuf,
    resolved_source:                  PathBuf,
    target_directory:                 PathBuf,
}

#[test]
fn shared_workspace_index_surface_exposes_resolutions_identities_and_revision_components()
-> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = workspace_index_api_fixture(&temp_dir)?;

    assert_revision_components(&fixture)?;
    assert_visible_target_workspace_ownership(&fixture);
    let workspace: &CargoWorkspaceView = fixture
        .cargo_workspace_index
        .workspaces()
        .next()
        .ok_or("workspace should be indexed")?;
    assert_workspace_root_resolutions(workspace, &fixture)?;
    assert_live_target_directory_resolution(workspace, &fixture.target_directory)?;
    Ok(())
}

fn workspace_index_api_fixture(temp_dir: &TempDir) -> TestResult<WorkspaceIndexApiFixture> {
    let checkout_root = temp_dir.path().join("checkout");
    let cargo_workspace_root = checkout_root.join("cargo-workspace");
    let resolved_member_root = cargo_workspace_root.join("crates/resolved-member");
    let resolved_source = resolved_member_root.join("src/main.rs");
    let unresolved_member_root = cargo_workspace_root.join("crates/unresolved-member");
    let unresolved_source = unresolved_member_root.join("src/main.rs");
    let target_directory = temp_dir.path().join("target");
    fs::create_dir_all(
        resolved_source
            .parent()
            .ok_or("source parent should exist")?,
    )?;
    fs::write(&resolved_source, "fn main() {}")?;

    let resolved_package_id = PackageId {
        repr: "resolved-member 0.1.0 (path+file:///resolved-member)".to_string(),
    };
    let unresolved_package_id = PackageId {
        repr: "unresolved-member 0.1.0 (path+file:///unresolved-member)".to_string(),
    };
    let mut packages = HashMap::new();
    packages.insert(
        resolved_package_id,
        package_record("resolved-member", &resolved_member_root, &resolved_source),
    );
    packages.insert(
        unresolved_package_id,
        package_record(
            "unresolved-member",
            &unresolved_member_root,
            &unresolved_source,
        ),
    );
    let mut metadata_store = WorkspaceMetadataStore::new();
    metadata_store.upsert(WorkspaceMetadata {
        declared_checkout_root: AbsolutePath::from(checkout_root.clone()),
        cargo_workspace_root: AbsolutePath::from(cargo_workspace_root.clone()),
        target_directory: AbsolutePath::from(target_directory.clone()),
        packages,
        fingerprint: ManifestFingerprint {
            manifest:       FileStamp {
                content_hash: [0_u8; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    });
    let accepted_cargo_metadata_revision = metadata_store.accepted_cargo_metadata_revision();
    let project_list_revision = ProjectListRevision::default();
    let cargo_workspace_index =
        CargoWorkspaceIndex::from_metadata_store(&metadata_store, project_list_revision);
    Ok(WorkspaceIndexApiFixture {
        accepted_cargo_metadata_revision,
        cargo_workspace_index,
        cargo_workspace_root,
        checkout_root,
        project_list_revision,
        resolved_member_root,
        resolved_source,
        target_directory,
    })
}

fn assert_visible_target_workspace_ownership(fixture: &WorkspaceIndexApiFixture) {
    let ownership: VisibleTargetWorkspaceOwnership<'_> =
        fixture.cargo_workspace_index.workspace_for_visible_target(
            &AbsolutePath::from(fixture.resolved_source.clone()),
            &AbsolutePath::from(fixture.resolved_member_root.clone()),
        );
    assert!(matches!(
        ownership,
        VisibleTargetWorkspaceOwnership::Indexed(workspace)
            if workspace.declared_checkout_root_path().as_path() == fixture.checkout_root
    ));
}

fn assert_revision_components(fixture: &WorkspaceIndexApiFixture) -> TestResult {
    let revision: CargoWorkspaceIndexRevision = match fixture.cargo_workspace_index.revision() {
        CargoWorkspaceIndexRevisionState::Accepted(revision) => revision,
        CargoWorkspaceIndexRevisionState::Uninitialized => {
            return Err("workspace index should have an accepted revision".into());
        },
    };
    assert_eq!(
        revision.accepted_cargo_metadata_revision(),
        fixture.accepted_cargo_metadata_revision
    );
    assert_eq!(
        revision.project_list_revision(),
        fixture.project_list_revision
    );
    Ok(())
}

fn assert_workspace_root_resolutions(
    workspace: &CargoWorkspaceView,
    fixture: &WorkspaceIndexApiFixture,
) -> TestResult {
    assert_eq!(
        workspace.declared_checkout_root_path().as_path(),
        fixture.checkout_root
    );
    assert_eq!(
        workspace.declared_workspace_root_path().as_path(),
        fixture.cargo_workspace_root
    );
    let checkout_resolution: &CanonicalPathResolution<CanonicalCheckoutRoot> =
        workspace.checkout_root_resolution();
    assert!(matches!(
        checkout_resolution,
        CanonicalPathResolution::Resolved(root)
            if root.path().as_path() == fixture.checkout_root.canonicalize()?
    ));
    let workspace_resolution: &CanonicalPathResolution<CanonicalWorkspaceRoot> =
        workspace.workspace_root_resolution();
    assert!(matches!(
        workspace_resolution,
        CanonicalPathResolution::Resolved(root)
            if root.path().as_path() == fixture.cargo_workspace_root.canonicalize()?
    ));
    Ok(())
}

fn assert_live_target_directory_resolution(
    workspace: &CargoWorkspaceView,
    target_directory: &Path,
) -> TestResult {
    assert_eq!(
        workspace.declared_target_directory_path().as_path(),
        target_directory
    );
    let unresolved_target_directory: CanonicalPathResolution<CanonicalTargetDirectory> =
        workspace.target_directory_resolution();
    assert!(matches!(
        unresolved_target_directory,
        CanonicalPathResolution::Unresolved
    ));
    fs::create_dir_all(target_directory)?;
    let resolved_target_directory: CanonicalPathResolution<CanonicalTargetDirectory> =
        workspace.target_directory_resolution();
    assert!(matches!(
        resolved_target_directory,
        CanonicalPathResolution::Resolved(target)
            if target.path().as_path() == target_directory.canonicalize()?
    ));
    Ok(())
}

fn package_record(name: &str, member_root: &Path, source_path: &Path) -> PackageRecord {
    PackageRecord {
        name:          name.to_string(),
        version:       Version::new(0, 1, 0),
        edition:       "2024".to_string(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(member_root.join("Cargo.toml")),
        targets:       vec![TargetRecord {
            name:              name.to_string(),
            kinds:             vec![TargetKind::Bin],
            src_path:          AbsolutePath::from(source_path.to_path_buf()),
            required_features: Vec::new(),
        }],
        publish:       PublishPolicy::Any,
    }
}
