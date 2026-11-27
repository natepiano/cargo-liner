"""Read the expected structure sites from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import sys
from contextlib import closing
from pathlib import Path
from shutil import copyfile
from typing import TypedDict

from oracle import (
    Reply, blob, commit, commit_with, copy_ledger, git, listed, python_refuses, reply, seed_ledger, write,
)
from scripts.style.style_ledger import Ledger


ROOT_ITEMS = "global:rust/module-roots.md"
SUBMODULES = "global:rust/submodule-names.md"
DEPENDENCIES = "global:rust/workspace-deps.md"
KANA = "global:rust/bevy-kana.md"
MEMBER_KANA = "local:docs/kana/style/bevy-kana.md"
FRESH = "structure.sqlite"
STEMS = ("helpers", "utils", "util", "common", "misc", "types", "model")
ROOT_STUTTER = "export_entry.rs"
ROOT_PACKAGE = """[package]
name = "root"
first = "an earlier key"
note = """
ROOT_PACKAGE += '"""\n\u00a0"quoted-dep" = visible to the line reader\n"""\n'
ROOT_PACKAGE += """
[dependencies]
first = "1"
"quoted-dep" = "1"
dotted.version = "2"
inherited = { workspace = true, version = "3" }
text_boolean = { workspace = "true", version = "3" }
path_only = { path = "parts/extra" }
number = 7
bevy = { workspace = true }

[dev-dependencies]
first = "2"
[build-dependencies]
build = { version = "1" }
"""
WORKSPACE = """[workspace]
members = [
    "crates/*", "crates/kana", "deep/**", "./parts//extra",
    "orphans/../orphans/child", "crates/missing", "vendor/kana", "crates/kana/src/nested",
]
exclude = ["crates/excluded", "deep/outer/invalid"]
"""
TOML_1_1_MANIFESTS = (
    ("trailing-comma", '[dependencies]\ndep = { version = "1", }\n'),
    ("inline-newline", '[dependencies]\ndep = {\nversion = "1"\n}\n'),
    ("escape", '[dependencies]\ndep = { version = "\\e" }\n'),
    ("short-time", 'published = 07:32\n[dependencies]\ndep = "1"\n'),
)
ITEM_SOURCE = """use crate::values;
mod section;
pub(crate) unsafe fn open() {
    fn nested() {}
    let value = "{";
}
/* } */
const TEXT: &str = "}";
macro_rules! spaced { () => {} }
macro_rules!joined { () => {} }
impl<T> Wrap<T> {}
struct View;
enum Mode {}
type Key = usize;
static COUNT: usize = 0;
trait Reader {}
"""
ENTRY_SOURCE = """fn main() {
    fn nested() {}
}
fn helper() {}
"""
KANA_SOURCE = """// pub type Comment = Velocity;
pub type Location = Position;
pub(crate) type Private = Velocity;
pub use hana_kana::Number;
pub type Other = VelocityExtra;
pub fn adjust() { let marker = "Displacement"; }
pub trait Cast: ToF32 {}
pub fn speed() -> Velocity { todo!() }
"""


class StructureSitesCase(TypedDict):
    name: str
    rule: str
    ledger: str
    expected: Reply


def case(name: str, rule: str, ledger: str, expected: Reply) -> StructureSitesCase:
    return {"name": name, "rule": rule, "ledger": ledger, "expected": expected}


def write_rules(root: Path, repo: Path) -> None:
    for rule, kind, tier in (
        (ROOT_ITEMS, "module_root_items", "B"),
        (SUBMODULES, "submodule_names", "B"),
        (DEPENDENCIES, "workspace_deps", "B"),
        (KANA, "bevy_kana_usage", "D"),
    ):
        write(root / "style", rule.removeprefix("global:"),
              f"---\ntier: {tier}\nfamily: modules\ncandidates:\n  kind: {kind}\n  surface: whole\n---\nStructure sites\n")
    write(repo, MEMBER_KANA.removeprefix("local:"), """---
tier: B
family: modules
candidates:
  kind: bevy_kana_usage
  surface: whole
---
Kana sites in this member
""")


def write_rust(repo: Path) -> None:
    write(repo, "src/layout/mod.rs", ITEM_SOURCE)
    write(repo, "src/main.rs", ENTRY_SOURCE)
    write(repo, "src/lib.rs", ENTRY_SOURCE)
    write(repo, "src/ordinary.rs", ITEM_SOURCE)
    write(repo, "src/ignored/lib.rs", ITEM_SOURCE)
    write(repo, "src/.hidden/lib.rs", ITEM_SOURCE)
    write(repo, "target/lib.rs", ITEM_SOURCE)
    write(repo, "src/only-main/main.rs", "fn main() {}\n")
    for path in (
        *(f"src/{stem}.rs" for stem in STEMS),
        "src/common/mod.rs", "src/net/net_worker.rs", "src/src_worker.rs", "repo_entry.rs", ROOT_STUTTER,
        "src/ignored/helpers.rs",
    ):
        write(repo, path, "// Module name fixture\n")
    (repo / "src/unreadable").mkdir()
    _ = (repo / "src/unreadable/lib.rs").write_bytes(b"fn unreadable() {} // \xff\n")

    write(repo, "crates/kana/src/lib.rs", KANA_SOURCE)
    write(repo, "crates/kana/src/ignored.rs", KANA_SOURCE)
    write(repo, "crates/kana/tests/check.rs", KANA_SOURCE)
    write(repo, "crates/kana/src/nested/src/lib.rs", KANA_SOURCE)
    write(repo, "crates/kana_no_lib/src/main.rs", KANA_SOURCE)
    write(repo, "vendor/kana/src/lib.rs", KANA_SOURCE)


def write_manifests(repo: Path) -> None:
    write(repo, "Cargo.toml", ROOT_PACKAGE + WORKSPACE)
    for directory, name, dependencies in (
        ("crates/kana", "kana", 'bevy = "0.1"\nhana_kana = { workspace = true }\n'),
        ("crates/kana_no_lib", "kana_no_lib", 'hana_kana = { workspace = true }\n'),
        ("crates/kana/src/nested", "kana_nested", 'hana_kana = { workspace = true }\n'),
        ("crates/excluded", "excluded", 'bevy = "0.1"\n'),
        ("crates/ignored", "ignored", 'bevy = "0.1"\n'),
        ("deep/outer/inner", "inner", 'engine = { package = "bevy", version = "0.1" }\n'),
        ("parts/extra", "extra", 'standalone = "0.1"\n'),
        ("orphans/child", "orphan", 'unlisted = "0.1"\n'),
        ("vendor/kana", "vendor_kana", 'hana_kana = { workspace = true }\n'),
    ):
        manifest = f'[package]\nname = "{name}"\n\n[dependencies]\n{dependencies}'
        write(repo, f"{directory}/Cargo.toml", manifest.replace("\n", "\r\n"))
    write(repo, "crates/not_crate.md", "A glob can match this file.\n")
    write(repo, "crates/missing/src/lib.rs", "// This member has no manifest.\n")
    write(repo, "deep/outer/invalid/Cargo.toml", "[invalid\n")
    write(repo, "crates/a/b/Cargo.toml", '[package]\nname = "nested"\n[dependencies]\ncross_slash = "1"\n')
    write(repo, "toml-1-1/control/Cargo.toml", '[package]\nname = "control"\n[dependencies]\ndep = "1"\n')
    for name, contents in TOML_1_1_MANIFESTS:
        write(repo, f"toml-1-1/{name}/Cargo.toml", contents)


def save_tip(root: Path, repo: Path) -> str:
    for key, value in (
        ("user.name", "Structure Sites Test"),
        ("user.email", "structure-sites@example.invalid"),
        ("core.hooksPath", "/dev/null"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)
    write_rules(root, repo)
    write_rust(repo)
    write_manifests(repo)
    write(repo, "rustfmt.toml", 'ignore = ["src/ignored", "crates/ignored", "crates/kana/src/ignored.rs"]\n')
    tip = commit(repo, "Save the structure sites input")
    write(repo, "src/main.rs", "fn dirty() {}\n")
    return tip


def check_module_roots(oracle: Ledger, repo: Path) -> None:
    rows = {row.path: (row.line, row.text) for row in listed(oracle, repo, ROOT_ITEMS)}
    assert rows["src/layout/mod.rs"] == (
        3, "9 top-level item(s) beyond mod/use: fn open, const TEXT, macro_rules! joined, impl, struct View, enum Mode, … (+3)",
    ), rows
    assert rows["src/main.rs"] == (4, "1 top-level item(s) beyond mod/use: fn helper"), rows
    assert rows["src/lib.rs"] == (1, "2 top-level item(s) beyond mod/use: fn main, fn helper"), rows
    assert set(rows).isdisjoint({
        "src/ordinary.rs", "src/ignored/lib.rs", "src/.hidden/lib.rs", "target/lib.rs",
        "src/only-main/main.rs", "src/unreadable/lib.rs",
    }), rows


def submodule_reply(oracle: Ledger, repo: Path) -> Reply:
    rows = listed(oracle, repo, SUBMODULES)
    assert [(row.path, row.line, row.text) for row in rows] == sorted([
        *((f"src/{stem}.rs", 1, f"anti-pattern module name `{stem}.rs`") for stem in STEMS),
        ("src/common/mod.rs", 1, "anti-pattern module directory `common/`"),
        ("src/net/net_worker.rs", 1, "`net/net_worker.rs` stutters the parent module name"),
        (ROOT_STUTTER, 1, "`export/export_entry.rs` stutters the parent module name"),
    ]), rows
    return reply(oracle, repo, SUBMODULES)


def check_dependencies(oracle: Ledger, repo: Path) -> None:
    rows = listed(oracle, repo, DEPENDENCIES)
    root = [(row.line, row.text) for row in rows if row.path == "Cargo.toml"]
    assert root == [
        (1, "[dependencies] dotted pins a version instead of workspace = true"),
        (3, "[dependencies] first pins a version instead of workspace = true"),
        (3, "[dev-dependencies] first pins a version instead of workspace = true"),
        (5, "[dependencies] quoted-dep pins a version instead of workspace = true"),
        (13, "[dependencies] text_boolean pins a version instead of workspace = true"),
        (21, "[build-dependencies] build pins a version instead of workspace = true"),
    ], root
    assert [(row.path, row.line) for row in rows if row.path != "Cargo.toml"] == [
        ("crates/excluded/Cargo.toml", 5),
        ("crates/kana/Cargo.toml", 5),
        ("deep/outer/inner/Cargo.toml", 5),
        ("parts/extra/Cargo.toml", 5),
    ], rows


def check_kana(oracle: Ledger, repo: Path) -> None:
    manifest_rows = [("Cargo.toml", 16), ("crates/excluded/Cargo.toml", 5)]
    source_rows = [
        (path, line)
        for path in (
            "crates/kana/src/lib.rs", "crates/kana/src/nested/src/lib.rs",
        )
        for line in (2, 4, 7, 8)
    ]
    rows = listed(oracle, repo, KANA)
    assert [(row.path, row.line) for row in rows] == manifest_rows + source_rows, rows
    assert rows[0].text == "bevy crate without a hana_kana dependency", rows
    assert rows[3].text == "pub use hana_kana::Number;", rows
    member = listed(oracle, repo, MEMBER_KANA)
    assert [(row.path, row.line) for row in member] == [
        (path, line) for path, line in source_rows if path.startswith("crates/kana/")
    ], member


def legacy_cases(root: Path, repo: Path, tip: str, reference: Path) -> list[StructureSitesCase]:
    with closing(sqlite3.connect(reference)) as connection, connection:
        _ = connection.execute("DELETE FROM refill_candidates")
        _ = connection.execute("DELETE FROM listed_sites WHERE rule = ? AND (path != ? OR line != ?)",
                               (DEPENDENCIES, "crates/excluded/Cargo.toml", 5))
    with Ledger(repo, reference) as oracle:
        rows = listed(oracle, repo, DEPENDENCIES)
        assert [(row.path, row.line) for row in rows] == [("crates/excluded/Cargo.toml", 5)], rows
        expected = reply(oracle, repo, DEPENDENCIES)

    old = root / "schema 47 structure.sqlite"
    seed_ledger(repo, old, tip, 47)
    with closing(sqlite3.connect(old)) as connection, connection:
        _ = connection.execute("ATTACH DATABASE ? AS legacy", (str(reference),))
        _ = connection.execute(
            "INSERT INTO listed_rules (rule, rule_blob, tip) SELECT rule, rule_blob, tip FROM legacy.listed_rules")
        _ = connection.execute(
            "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
            + "SELECT rule, rule_blob, tip, path, line, text FROM legacy.listed_sites")
    migrated = root / "migrated structure.sqlite"
    copy_ledger(old, migrated)
    with Ledger(repo, migrated) as oracle:
        assert reply(oracle, repo, DEPENDENCIES) == expected
    return [
        case("schema 51 keeps saved dependency rows", DEPENDENCIES, reference.name, expected),
        case("schema 47 keeps saved dependency rows", DEPENDENCIES, old.name, expected),
    ]


def manifest_variant(root: Path, repo: Path, tip: str, label: str, source: str) -> Path:
    variant = commit_with(repo, tip, "Cargo.toml", f"100644 blob {blob(repo, source.encode())}", label)
    ledger = root / f"{label}.sqlite"
    seed_ledger(repo, ledger, variant)
    return ledger


def workspace_cases(root: Path, repo: Path, tip: str) -> list[StructureSitesCase]:
    cases: list[StructureSitesCase] = []
    for label, source, root_present in (
        ("workspace without root package", WORKSPACE + '\n[dependencies]\nextra = "1"\n', False),
        ("package without workspace", ROOT_PACKAGE, True),
    ):
        ledger = manifest_variant(root, repo, tip, label, source)
        reference = root / "workspace oracle.sqlite"
        copy_ledger(ledger, reference)
        with Ledger(repo, reference) as oracle:
            dependencies = listed(oracle, repo, DEPENDENCIES)
            expected_paths = [] if root_present else [
                "crates/excluded/Cargo.toml", "crates/kana/Cargo.toml", "deep/outer/inner/Cargo.toml", "parts/extra/Cargo.toml",
            ]
            assert [row.path for row in dependencies] == expected_paths, dependencies
            cases.append(case(label, DEPENDENCIES, ledger.name, reply(oracle, repo, DEPENDENCIES)))
            if root_present:
                kana = listed(oracle, repo, KANA)
                assert [(row.path, row.line) for row in kana] == [("Cargo.toml", 16)], kana
                cases.append(case(label, KANA, ledger.name, reply(oracle, repo, KANA)))
    return cases


def workspace_parity_cases(root: Path, repo: Path, tip: str) -> list[StructureSitesCase]:
    cases: list[StructureSitesCase] = []
    # The catalog honors exclude. The workspace_deps generator reads the excluded manifests.
    toml_members = ", ".join(f'"toml-1-1/{name}"' for name, _ in TOML_1_1_MANIFESTS)
    for label, source, path, line in (
        ("negated class matches a slash", '[workspace]\nmembers = ["crates/a[!x]*"]\n',
         "crates/a/b/Cargo.toml", 4),
        ("star pattern matches the workspace root", '[workspace]\nmembers = ["***"]\n[dependencies]\nroot_pin = "1"\n',
         "Cargo.toml", 4),
        ("TOML 1.1 manifests add no pins",
         f'[workspace]\nmembers = ["toml-1-1/control", {toml_members}]\nexclude = ["toml-1-1"]\n',
         "toml-1-1/control/Cargo.toml", 4),
        ("absolute member inside the input root", '[workspace]\nmembers = ["/export/crates/a/b"]\n',
         "crates/a/b/Cargo.toml", 4),
    ):
        ledger = manifest_variant(root, repo, tip, label, source)
        reference = root / "workspace parity oracle.sqlite"
        copy_ledger(ledger, reference)
        with Ledger(repo, reference) as oracle:
            rows = listed(oracle, repo, DEPENDENCIES)
            assert [(row.path, row.line) for row in rows] == [(path, line)], (label, rows)
            cases.append(case(label, DEPENDENCIES, ledger.name, reply(oracle, repo, DEPENDENCIES)))

    reason = "is not in the subpath of"
    for label, outside_member in (
        ("absolute member outside the input root", "/outside-workspace/member"),
        ("absolute member in the request repository", str(repo.absolute() / "crates/a/b")),
    ):
        outside_manifest = f"{outside_member}/Cargo.toml"
        source = f"[workspace]\nmembers = {json.dumps([outside_member], ensure_ascii=False)}\n"
        ledger = manifest_variant(root, repo, tip, label, source)
        python_refuses(root, ledger.name, "repo", DEPENDENCIES, ValueError,
                       f"{outside_manifest!r} {reason} '/export'")
        cases.append(case(label, DEPENDENCIES, ledger.name, {"refusal": [f'{reason} "/export"', outside_manifest]}))
    return cases


def unreadable_manifest(root: Path, repo: Path, tip: str) -> StructureSitesCase:
    manifest = "deep/outer/invalid/Cargo.toml"
    bad_blob = blob(repo, b"[package]\nname = \"bad \xff\"\n")
    broken = commit_with(repo, tip, manifest, f"100644 blob {bad_blob}", "Save unreadable manifest")
    staging = root / "manifest staging.sqlite"
    ledger = root / "unreadable manifest.sqlite"
    copy_ledger(root / FRESH, staging)
    with closing(sqlite3.connect(staging)) as connection:
        mode = connection.execute("PRAGMA journal_mode = WAL").fetchone()
        assert mode == ("wal",), mode
        with connection:
            _ = connection.execute("UPDATE sync SET tip = ?", (broken,))
        for suffix in ("", "-wal"):
            _ = copyfile(f"{staging}{suffix}", f"{ledger}{suffix}")
        python_refuses(root, staging.name, "repo", DEPENDENCIES, UnicodeDecodeError, "can't decode byte 0xff")
    return case("workspace dependencies refuse an unreadable manifest", DEPENDENCIES, ledger.name,
                {"refusal": ["manifest is not UTF-8", manifest]})


def main(root: Path) -> None:
    repo = root / "repo"
    tip = save_tip(root, repo)
    seed_ledger(repo, root / FRESH, tip)
    reference = root / "legacy structure.sqlite"
    copy_ledger(root / FRESH, reference)
    with Ledger(repo, reference) as oracle:
        check_module_roots(oracle, repo)
        check_dependencies(oracle, repo)
        check_kana(oracle, repo)
        cases = [case(rule, rule, FRESH, reply(oracle, repo, rule))
                 for rule in (ROOT_ITEMS, DEPENDENCIES, KANA, MEMBER_KANA)]
        cases.append(case(SUBMODULES, SUBMODULES, FRESH, submodule_reply(oracle, repo)))
    cases.extend(legacy_cases(root, repo, tip, reference))
    cases.extend(workspace_cases(root, repo, tip))
    cases.extend(workspace_parity_cases(root, repo, tip))
    cases.append(unreadable_manifest(root, repo, tip))
    print(json.dumps(cases))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
