"""Create a ledger and read its footprints with pinned Python."""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, TypedDict

from scripts.style import style_ledger, style_plan
from scripts.style.git_process import GitReadFailed


STAMP = "2026-01-01T00:00:00+00:00"
MISSING_BLOB = "f" * 40
UNKNOWN_FINDING = "f" * 16
LOCK = "app/src/lock.rs"
QUALIFIED = "app/src/qualified.rs"
CR_NAME = "docs/word\rboundary.md"
CR_OWNER = "app/src/owner\rcopy.rs"
WARNING_MATCH = "only_with_unreadable_blob.md"
DOTLESS_ITEM = "app/src/ıtem.rs"
DOTLESS_OWNER = "app/src/item_owner.rs"


@dataclass(frozen=True)
class FindingCase:
    label: str
    path: str
    kind: str
    before: str
    after: str


class FootprintReading(TypedDict):
    files: tuple[str, ...]
    uncertain: tuple[str, ...]


class FootprintRow(TypedDict):
    case: str
    id: str
    footprint: FootprintReading | None


class AvailableFifo(TypedDict):
    available: list[FootprintRow]


def git(repo: Path, *args: str, input_bytes: bytes = b"") -> str:
    """Run Git with the fixture settings."""
    output = subprocess.run(
        ("git", "-C", str(repo), *args), input=input_bytes, capture_output=True, check=True,
    )
    return output.stdout.decode().strip()


def write(repo: Path, path: str, content: str | bytes) -> None:
    """Write one source file."""
    file = repo / path
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_bytes(content.encode() if isinstance(content, str) else content)


def set_library(repo: Path, library: str) -> None:
    """Set the checkout library name."""
    write(repo, "app/Cargo.toml", f'[package]\nname = "unused-package"\n[lib]\nname = "{library}"\n')


def source_commits(repo: Path) -> tuple[str, str]:
    """Commit base and tip versions of the search domain."""
    for option, value in (
        ("user.name", "Footprint Test"),
        ("user.email", "footprint@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("maintenance.auto", "false"),
    ):
        git(repo, "config", option, value)
    set_library(repo, "committed-lib")
    for path, content in (
        (".gitignore", "vendor/\n"),
        ("app/src/lib.rs", "pub mod lock;\npub mod layout;\npub mod qualified;\n"),
        (LOCK, "pub fn lock() {}\npub fn current() { crate::lock::lock(); }\n"),
        ("app/src/layout.rs", "pub fn layout() {}\n"),
        (DOTLESS_ITEM, "pub fn current() {}\n"),
        (DOTLESS_OWNER, "mod ıtem;\n"),
        (QUALIFIED, "pub fn shared() {}\n"),
        ("app/src/imports.rs", "use crate::lock::current;\n"),
        ("app/src/comments.rs", "//! See [`crate::lock::current`].\n"),
        ("app/src/inline.rs", 'mod inner {\n    #[path = "../lock.rs"]\n    mod lock;\n}\n'),
        ("app/src/unread_tokens.rs", "/* lock\n"),
        ("app/src/uses_shared.rs", "use SharedType::shared;\n"),
        ("app/src/elsewhere.rs", '#[path = "another.rs"]\nmod lock;\n'),
        ("app/src/conditional.rs", '#[cfg_attr(test, path = "lock.rs")]\nmod lock;\n'),
        ("app/src/escaped.rs", r'#[path = "else\where.rs"]' + "\nmod hidden;\n"),
        ("app/src/unread.rs", '#[path = r#"different.rs"#]\nmod another;\n'),
        ("app/src/versioned.rs", "use crate::lock::current;\n"),
        ("app/src/base_only.rs", "use crate::lock::current;\n"),
        ("app/src/bytes.rs", b"// \xff shared\n"),
        ("docs/shared.md", "See SharedType::shared.\n"),
        ("docs/legacy.md", "See SharedType::shared.\n"),
        (CR_NAME, "See SharedType::shared.\n"),
        ("integration/attribute.rs", '#[path = "../app/src/lock.rs"]\nmod alias;\n'),
        ("integration/local.rs", "use local_lib::lock::current;\n"),
        ("integration/committed.rs", "use committed_lib::lock::current;\n"),
        ("integration/package.rs", "use unused_package::lock::current;\n"),
        ("vendor/case.rs", "// éshared\n"),
    ):
        write(repo, path, content)
    git(repo, "add", "-f", ".")
    git(repo, "commit", "-qm", "Write the footprint base")
    base = git(repo, "rev-parse", "HEAD")
    git(repo, "rm", "-q", "docs/legacy.md", "app/src/base_only.rs")
    write(repo, "app/src/versioned.rs", "pub fn lock() {}\n")
    git(repo, "add", "app/src/versioned.rs")
    git(repo, "commit", "-qm", "Write the synced tip")
    return base, git(repo, "rev-parse", "HEAD")


def seed_ledger(
    repo: Path, database: Path, tip: str, cases: dict[str, FindingCase],
) -> None:
    """Apply the real ledger migrations and record the fixture findings."""
    with style_ledger.Ledger(repo, database):
        pass
    with closing(sqlite3.connect(database)) as connection, connection:
        connection.execute("INSERT INTO sync (id, tip, synced_at) VALUES (1, ?, ?)", (tip, STAMP))
        connection.executemany(
            """INSERT INTO held_findings
            (finding_id, rule, path, line, text, kind, before, after, rule_blob, ticket, state, proposal,
             created_at, updated_at, anchor_commit)
            VALUES (?, 'global:rust/footprint-fixture.md', ?, 1, 'footprint fixture', ?, ?, ?,
                    ?, 'absent', 'unfiled', '', ?, ?, ?)""",
            ((identity, row.path, row.kind, row.before, row.after, "1" * 40, STAMP, STAMP, tip)
             for identity, row in cases.items()),
        )
    plain_id, qualified_id, *_ = cases
    with style_ledger.Ledger(repo, database) as ledger:
        for path in ("planned/z.rs", "planned/a.rs", "planned/z.rs", QUALIFIED):
            ledger.finding_need(plain_id, path)
        for path in ("planned/z.rs", "docs/shared.md", QUALIFIED):
            ledger.finding_need(qualified_id, path)


def footprint_rows(
    repo: Path, database: Path, base: style_plan.UnitBase, cases: dict[str, FindingCase], requested: list[str],
) -> list[FootprintRow]:
    """Read the footprints in request order with the fixed Git search."""
    with style_ledger.Ledger(repo, database, access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
        findings = {row.finding_id: row for row in ledger.held_findings(style_ledger.AllHeldFindings())}
        edited = style_plan.EditedFiles(repo, ledger, base)
        answers: list[FootprintRow] = []
        for identity in requested:
            if identity not in findings:
                answers.append({"case": "unknown", "id": identity, "footprint": None})
                continue
            row = findings[identity]
            footprint = edited.footprint(row)
            answers.append({"case": cases[identity].label, "id": identity,
                            "footprint": {"files": footprint.files, "uncertain": footprint.uncertain}})
        return answers


def missing_candidate_base(repo: Path, tip: str) -> str:
    """Add a missing Rust blob and a readable match to a separate tree."""
    entries = git(repo, "ls-tree", tip).encode() + b"\n"
    missing = f"100644 blob {MISSING_BLOB}\tmissing_candidate.rs\n".encode()
    blob = git(repo, "hash-object", "-w", "--stdin", input_bytes=b"See SharedType::shared.\n")
    match = f"100644 blob {blob}\t{WARNING_MATCH}\n".encode()
    tree = git(repo, "mktree", "--missing", input_bytes=entries + missing + match)
    return git(repo, "commit-tree", tree, "-p", tip, "-m", "Record a missing candidate blob")


def missing_owner_base(repo: Path, tip: str) -> str:
    """Put a carriage return in a module owner's path."""
    write(repo, CR_OWNER, "use crate::lock::current;\n")
    git(repo, "add", "--", CR_OWNER)
    git(repo, "commit", "-qm", "Add a module owner with a carriage return in its path")
    revision = git(repo, "rev-parse", "HEAD")
    git(repo, "checkout", "-q", "--detach", tip)
    return revision


def fifo_manifest_rows(
    root: Path, database: Path, cases: dict[str, FindingCase], move: str, tip: str,
) -> Literal["unsupported"] | AvailableFifo:
    """Read a footprint while the checkout manifest is a FIFO."""
    if not hasattr(os, "mkfifo"):
        return "unsupported"
    repo = root / "repo"
    manifest = repo / "app/Cargo.toml"
    manifest.unlink()
    os.mkfifo(manifest)
    rows = footprint_rows(repo, database, style_plan.NoUnitBranch(), cases, [move])
    footprint = rows[0]["footprint"]
    assert footprint is not None, rows
    assert "app/src/imports.rs" in footprint["files"], rows
    assert "integration/committed.rs" not in footprint["files"], rows
    manifest.rename(root / "manifest.fifo")
    git(repo, "checkout", "-q", tip, "--", "app/Cargo.toml")
    return {"available": rows}


def main(root: Path) -> None:
    """Print the expected replies for each checkout and base."""
    repo = root / "repo"
    database = root / "ledger #1.sqlite"
    base, tip = source_commits(repo)
    rows = (
        FindingCase("recorded needs", QUALIFIED, "rename", "plain_old", "plain_new"),
        FindingCase("qualified", QUALIFIED, "rename", "SharedType::shared", "SharedType::common"),
        FindingCase("file move", LOCK, "rename", "", "Rename lock.rs to guard.rs"),
        FindingCase("restructure", "app/src/layout.rs", "restructure", "", "Split the layout"),
        FindingCase("unsupported move", LOCK, "rename", "", "Rename lock.rs to other/guard.rs"),
        FindingCase("missing candidate", QUALIFIED, "rename", "SharedType::missing_word", "SharedType::new_word"),
        FindingCase("dotless i file move", DOTLESS_ITEM, "rename", "", "Rename ıtem.rs to newer.rs"),
    )
    cases = {f"{index:016x}": case for index, case in enumerate(rows, 1)}
    seed_ledger(repo, database, tip, cases)
    plain, qualified, move, restructure, unsupported, missing, dotless = cases
    requested = [move, UNKNOWN_FINDING, qualified, unsupported, plain, restructure, dotless, move]
    synced = footprint_rows(repo, database, style_plan.NoUnitBranch(), cases, requested)
    with_base = footprint_rows(repo, database, style_plan.UnitBranchTip(base), cases, requested)
    equal_base = footprint_rows(repo, database, style_plan.UnitBranchTip(tip), cases, requested)
    assert synced == equal_base
    dotless_proof, = (row["footprint"] for row in synced if row["id"] == dotless)
    assert dotless_proof is not None and DOTLESS_OWNER in dotless_proof["files"], dotless_proof
    broken_base = missing_candidate_base(repo, tip)
    try:
        footprint_rows(repo, database, style_plan.UnitBranchTip(broken_base), cases, [plain, missing])
    except GitReadFailed as error:
        assert MISSING_BLOB in error.stderr, error
    else:
        raise AssertionError("Python accepted the missing candidate blob")

    search = subprocess.run(
        ("git", "-C", str(repo), "grep", "-l", "-z", "-w", "-F", "-e", "SharedType::shared",
         broken_base, "--", "*.rs", "*.md"),
        check=True, capture_output=True,
    )
    assert MISSING_BLOB.encode() in search.stderr and WARNING_MATCH.encode() in search.stdout, search
    warning_match = footprint_rows(repo, database, style_plan.UnitBranchTip(broken_base), cases, [qualified])
    warning_proof = warning_match[0]["footprint"]
    assert warning_proof is not None and WARNING_MATCH in warning_proof["files"], warning_match

    owner_base = missing_owner_base(repo, tip)
    try:
        footprint_rows(repo, database, style_plan.UnitBranchTip(owner_base), cases, [plain, move])
    except style_ledger.CheckCouldNotRun as error:
        assert "no blob for " + CR_OWNER in str(error), repr(str(error))
    else:
        raise AssertionError("Python accepted the module owner with a carriage return in its path")

    set_library(repo, "local-lib")
    changed_library = footprint_rows(repo, database, style_plan.NoUnitBranch(), cases, [move])
    (repo / "app/src/lib.rs").unlink()
    no_root = footprint_rows(repo, database, style_plan.NoUnitBranch(), cases, [move])
    git(repo, "checkout", "-q", tip, "--", "app/Cargo.toml", "app/src/lib.rs")
    fifo_manifest = fifo_manifest_rows(root, database, cases, move, tip)
    print(json.dumps({
        "tip": tip, "base": base, "synced": synced, "with_base": with_base, "plain": plain,
        "changed_library": changed_library, "no_root": no_root, "broken_base": broken_base,
        "missing_blob": MISSING_BLOB, "missing_finding": missing,
        "missing_owner": {"base": owner_base, "finding": move, "diagnostic": "no blob for " + CR_OWNER},
        "warning_match": warning_match, "fifo_manifest": fifo_manifest,
    }))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
