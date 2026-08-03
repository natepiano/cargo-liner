use std::collections::BTreeMap;
use std::collections::HashMap;
use std::error::Error;
use std::fs;

use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;

use crate::project::AbsolutePath;
use crate::project::AcceptedCargoMetadataRevision;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalMemberRoot;
use crate::project::CanonicalPackageOwnership;
use crate::project::CanonicalPathResolution;
use crate::project::CanonicalTargetDirectory;
use crate::project::CanonicalTargetOwnership;
use crate::project::CanonicalTargetSource;
use crate::project::CanonicalWorkspaceRoot;
use crate::project::CargoPackageIdentity;
use crate::project::CargoTargetIdentity;
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
use crate::project::VisibleProjectWorkspaceOwnership;
use crate::project::VisibleTargetWorkspaceOwnership;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct WorkspaceIndexApiFixture {
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    cargo_workspace_index:            CargoWorkspaceIndex,
    cargo_workspace_root:             std::path::PathBuf,
    checkout_root:                    std::path::PathBuf,
    project_list_revision:            ProjectListRevision,
    resolved_member_root:             std::path::PathBuf,
    resolved_package_id:              PackageId,
    resolved_source:                  std::path::PathBuf,
    target_directory:                 std::path::PathBuf,
    unresolved_member_root:           std::path::PathBuf,
    unresolved_package_id:            PackageId,
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
    assert_package_identities(workspace, &fixture)?;
    assert_live_target_directory_resolution(workspace, &fixture.target_directory)?;
    Ok(())
}

#[test]
fn project_scope_queries_preserve_exact_and_ambiguous_workspace_ownership() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let first_checkout_root = temp_dir.path().join("first-checkout");
    let second_checkout_root = temp_dir.path().join("second-checkout");
    let shared_workspace_root = temp_dir.path().join("shared-workspace");
    fs::create_dir_all(&first_checkout_root)?;
    fs::create_dir_all(&second_checkout_root)?;
    fs::create_dir_all(&shared_workspace_root)?;
    let mut workspace_metadata_store = WorkspaceMetadataStore::new();
    workspace_metadata_store.upsert(scope_workspace_metadata(
        &first_checkout_root,
        &shared_workspace_root,
    ));
    workspace_metadata_store.upsert(scope_workspace_metadata(
        &second_checkout_root,
        &shared_workspace_root,
    ));
    let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
        &workspace_metadata_store,
        ProjectListRevision::default(),
    );

    assert!(matches!(
        cargo_workspace_index.workspace_for_visible_project(&AbsolutePath::from(
            first_checkout_root.as_path()
        )),
        VisibleProjectWorkspaceOwnership::Indexed(workspace)
            if workspace.declared_checkout_root_path().as_path() == first_checkout_root
    ));
    assert!(matches!(
        cargo_workspace_index.workspace_for_workspace_root(&AbsolutePath::from(
            first_checkout_root.as_path()
        )),
        VisibleProjectWorkspaceOwnership::Indexed(workspace)
            if workspace.declared_checkout_root_path().as_path() == first_checkout_root
    ));
    assert!(matches!(
        cargo_workspace_index
            .workspace_for_visible_project(&AbsolutePath::from(shared_workspace_root.as_path())),
        VisibleProjectWorkspaceOwnership::Ambiguous
    ));
    assert!(matches!(
        cargo_workspace_index
            .workspace_for_workspace_root(&AbsolutePath::from(shared_workspace_root.as_path())),
        VisibleProjectWorkspaceOwnership::Ambiguous
    ));
    Ok(())
}

#[test]
fn canonical_package_queries_preserve_exact_ambiguous_and_unindexed_ownership() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let shared_member_root = temp_dir.path().join("shared-member");
    let shared_source = shared_member_root.join("src/main.rs");
    let sole_member_root = temp_dir.path().join("sole-member");
    let sole_source = sole_member_root.join("src/main.rs");
    for source in [&shared_source, &sole_source] {
        fs::create_dir_all(source.parent().ok_or("source parent should exist")?)?;
        fs::write(source, "fn main() {}")?;
    }

    let sole_package_id = PackageId {
        repr: "sole-member 0.1.0 (path+file:///sole-member)".to_string(),
    };
    let mut first_packages = HashMap::new();
    first_packages.insert(
        sole_package_id.clone(),
        package_record("sole-member", &sole_member_root, &sole_source),
    );
    first_packages.insert(
        PackageId {
            repr: "shared-member 0.1.0 (path+file:///first/shared-member)".to_string(),
        },
        package_record("shared-member", &shared_member_root, &shared_source),
    );
    let mut second_packages = HashMap::new();
    second_packages.insert(
        PackageId {
            repr: "shared-member 0.1.0 (path+file:///second/shared-member)".to_string(),
        },
        package_record("shared-member", &shared_member_root, &shared_source),
    );

    let mut workspace_metadata_store = WorkspaceMetadataStore::new();
    for (checkout_name, packages) in [("first", first_packages), ("second", second_packages)] {
        let checkout_root = temp_dir.path().join(checkout_name);
        let mut workspace_metadata =
            scope_workspace_metadata(&checkout_root, checkout_root.as_path());
        workspace_metadata.packages = packages;
        workspace_metadata_store.upsert(workspace_metadata);
    }
    let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
        &workspace_metadata_store,
        ProjectListRevision::default(),
    );

    let canonical_sole_member_root = AbsolutePath::from(fs::canonicalize(&sole_member_root)?);
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_member_root(&canonical_sole_member_root),
        CanonicalPackageOwnership::Indexed(package) if package.package_id() == &sole_package_id
    ));
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_target_source(&AbsolutePath::from(
            fs::canonicalize(&sole_source)?
        )),
        CanonicalTargetOwnership::Indexed(package) if package.package_id() == &sole_package_id
    ));

    // Two indexed workspaces declare the same member root and source file, so
    // neither query may name one of them.
    let canonical_shared_member_root = AbsolutePath::from(fs::canonicalize(&shared_member_root)?);
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_member_root(&canonical_shared_member_root),
        CanonicalPackageOwnership::Ambiguous
    ));
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_target_source(&AbsolutePath::from(
            fs::canonicalize(&shared_source)?
        )),
        CanonicalTargetOwnership::Ambiguous
    ));

    let unindexed_member_root = AbsolutePath::from(temp_dir.path().join("unindexed-member"));
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_member_root(&unindexed_member_root),
        CanonicalPackageOwnership::NotIndexed
    ));
    assert!(matches!(
        cargo_workspace_index.package_for_canonical_target_source(&AbsolutePath::from(
            unindexed_member_root.as_path().join("src/main.rs")
        )),
        CanonicalTargetOwnership::NotIndexed
    ));
    Ok(())
}

fn workspace_index_api_fixture(
    temp_dir: &tempfile::TempDir,
) -> TestResult<WorkspaceIndexApiFixture> {
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
        resolved_package_id.clone(),
        package_record("resolved-member", &resolved_member_root, &resolved_source),
    );
    packages.insert(
        unresolved_package_id.clone(),
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
        resolved_package_id,
        resolved_source,
        target_directory,
        unresolved_member_root,
        unresolved_package_id,
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

fn assert_package_identities(
    workspace: &CargoWorkspaceView,
    fixture: &WorkspaceIndexApiFixture,
) -> TestResult {
    let resolved_package: &CargoPackageIdentity = workspace
        .packages()
        .find(|package| package.package_id() == &fixture.resolved_package_id)
        .ok_or("resolved package should be indexed")?;
    assert_eq!(resolved_package.package_id(), &fixture.resolved_package_id);
    assert_eq!(
        resolved_package.declared_member_root_path().as_path(),
        fixture.resolved_member_root
    );
    let member_resolution: &CanonicalPathResolution<CanonicalMemberRoot> =
        resolved_package.member_root_resolution();
    assert!(matches!(
        member_resolution,
        CanonicalPathResolution::Resolved(root)
            if root.path().as_path() == fixture.resolved_member_root.canonicalize()?
    ));
    let target_identity: &CargoTargetIdentity = resolved_package
        .targets()
        .next()
        .ok_or("resolved target should be indexed")?;
    assert_eq!(target_identity.name(), "resolved-member");
    assert_eq!(target_identity.kinds(), &[TargetKind::Bin]);
    assert!(target_identity.required_features().is_empty());
    let declared_source_path: &AbsolutePath = target_identity.declared_source_path();
    assert_eq!(declared_source_path.as_path(), fixture.resolved_source);
    let source_resolution: &CanonicalPathResolution<CanonicalTargetSource> =
        target_identity.canonical_source_resolution();
    assert!(matches!(
        source_resolution,
        CanonicalPathResolution::Resolved(source)
            if source.path().as_path() == fixture.resolved_source.canonicalize()?
    ));

    let unresolved_package: &CargoPackageIdentity = workspace
        .packages()
        .find(|package| package.package_id() == &fixture.unresolved_package_id)
        .ok_or("unresolved package should be indexed")?;
    assert_eq!(
        unresolved_package.declared_member_root_path().as_path(),
        fixture.unresolved_member_root
    );
    assert!(matches!(
        unresolved_package.member_root_resolution(),
        CanonicalPathResolution::Unresolved
    ));
    let unresolved_target: &CargoTargetIdentity = unresolved_package
        .targets()
        .next()
        .ok_or("unresolved target should be indexed")?;
    let declared_source_path: &AbsolutePath = unresolved_target.declared_source_path();
    assert_eq!(
        declared_source_path.as_path(),
        fixture.unresolved_member_root.join("src/main.rs")
    );
    assert!(matches!(
        unresolved_target.canonical_source_resolution(),
        CanonicalPathResolution::Unresolved
    ));
    Ok(())
}

fn assert_live_target_directory_resolution(
    workspace: &CargoWorkspaceView,
    target_directory: &std::path::Path,
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

fn package_record(
    name: &str,
    member_root: &std::path::Path,
    source_path: &std::path::Path,
) -> PackageRecord {
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

fn scope_workspace_metadata(
    checkout_root: &std::path::Path,
    cargo_workspace_root: &std::path::Path,
) -> WorkspaceMetadata {
    WorkspaceMetadata {
        declared_checkout_root:   AbsolutePath::from(checkout_root),
        cargo_workspace_root:     AbsolutePath::from(cargo_workspace_root),
        target_directory:         AbsolutePath::from(checkout_root.join("target")),
        packages:                 HashMap::new(),
        fingerprint:              ManifestFingerprint {
            manifest:       FileStamp {
                content_hash: [0_u8; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    }
}
