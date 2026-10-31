use crate::support::*;

#[test]
fn shared_source_visibility_keeps_callers_in_every_mount() {
    let temp = tempdir().expect("create shared visibility fixture");
    fs::create_dir_all(temp.path().join("src/support/nested"))
        .expect("create nested support directory");
    fs::create_dir_all(temp.path().join("src/internal/nested"))
        .expect("create public-type fixture mounts");
    for (path, source) in [
        (
            "Cargo.toml",
            "[package]\nname = \"shared_visibility_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ntest = false\nbench = false\n\n[profile.dev]\ndebug = 0\nincremental = false\n",
        ),
        (
            "mend.toml",
            "[diagnostics]\nprefer_module_import = false\nreview_pub_mod = false\n\n[visibility]\npub_in_path = \"permitted\"\n",
        ),
        (
            "src/lib.rs",
            "#![no_std]\nmod support;\n#[path = \"shared.rs\"]\nmod test_pointer;\nmod local;\n#[path = \"public_item.rs\"]\npub mod api;\nmod internal;\n#[path = \"helper.rs\"]\nmod a;\n#[path = \"helper.rs\"]\nmod b;\npub trait PublicTrait { type Output; }\npub struct PublicImpl;\nimpl PublicTrait for PublicImpl { type Output = api::Item; }\npub fn run() -> u8 { local::call(); let _ = internal::nested::hidden::Item; a::helper(); support::call() + test_pointer::parent_caller() + test_pointer::cross_mount() }\n",
        ),
        (
            "src/support/mod.rs",
            "mod nested;\npub(crate) fn call() -> u8 { nested::shared::parent_caller() }\n",
        ),
        (
            "src/support/nested/mod.rs",
            "#[path = \"../../shared.rs\"]\npub(super) mod shared;\n",
        ),
        (
            "src/shared.rs",
            "pub(crate) fn parent_caller() -> u8 { cross_mount() }\npub(crate) fn cross_mount() -> u8 { 7 }\n",
        ),
        ("src/public_item.rs", "pub struct Item;\n"),
        ("src/helper.rs", "pub fn helper() {}\n"),
        ("src/internal/mod.rs", "pub(crate) mod nested;\n"),
        (
            "src/internal/nested/mod.rs",
            "#[path = \"../../public_item.rs\"]\npub(crate) mod hidden;\n",
        ),
        (
            "src/local.rs",
            "pub(crate) fn call() { local_only(); }\npub(crate) fn local_only() {}\n",
        ),
    ] {
        fs::write(temp.path().join(path), source).expect("write visibility fixture");
    }

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
        .arg("--fix")
        .arg("--lib")
        .output()
        .expect("fix shared visibility fixture");
    assert!(
        output.status.success(),
        "shared mounts must compile after --fix:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let shared = fs::read_to_string(temp.path().join("src/shared.rs"))
        .expect("read shared source after fix");
    assert!(shared.contains("pub(crate) fn parent_caller()"), "{shared}");
    assert!(shared.contains("pub(crate) fn cross_mount()"), "{shared}");
    let public_item = fs::read_to_string(temp.path().join("src/public_item.rs"))
        .expect("read publicly exposed shared item");
    assert!(public_item.contains("pub struct Item;"), "{public_item}");
    let helper = fs::read_to_string(temp.path().join("src/helper.rs"))
        .expect("read helper used through one mount");
    assert!(helper.contains("pub fn helper()"), "{helper}");
    let local = fs::read_to_string(temp.path().join("src/local.rs"))
        .expect("read single-mount source after fix");
    assert!(local.contains("\nfn local_only()"), "{local}");
}
