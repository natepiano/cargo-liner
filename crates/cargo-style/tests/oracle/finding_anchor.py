"""Build a real Python ledger and print its anchor answers for the Rust test."""

from __future__ import annotations

import json
import shutil
import sqlite3
import subprocess
import sys
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path

from scripts.style.style_ledger import AllHeldFindings, FindingAnchor, Ledger, LedgerAccess, MIGRATIONS


RULE = "global:rust/finding-anchor-fixture.md"
STAMP = "2026-01-01T00:00:00+00:00"
MISSING_COMMIT = "f" * 40
UNKNOWN_FINDING = "f" * 16
TARGET = "// stored coordinate"
REVISION = 19


@dataclass(frozen=True)
class FindingCase:
    """The stored fields that select one source history."""

    name: str
    path: str
    text: str = TARGET
    line: int = 2
    ticket: str = "judging"
    anchor: str = ""
    follow: str = ""


def git(repo: Path, *arguments: str) -> str:
    """Run Git with the fixture environment."""
    return subprocess.run(
        ("git", "-C", str(repo), *arguments), capture_output=True, check=True, text=True,
    ).stdout.strip()


def source_commits(repo: Path) -> tuple[str, str]:
    """Commit the base and checkpoint with each source case on its own path."""
    git(repo, "config", "user.name", "Finding Anchor Test")
    git(repo, "config", "user.email", "anchor@example.invalid")
    git(repo, "config", "commit.gpgsign", "false")
    git(repo, "config", "core.hooksPath", "/dev/null")
    git(repo, "config", "maintenance.auto", "false")
    (repo / "src").mkdir()
    original = f"// lead\n{TARGET}\n// tail\n"
    for name in ("same", "base", "stable", "moved", "invalid"):
        (repo / "src" / f"{name}.rs").write_text(original, encoding="utf-8")
    (repo / "src/checkpoint.rs").write_text("// lead\n// absent\n", encoding="utf-8")
    patch_lines = f"// first\n// second\n{TARGET}\n// fourth\n// fifth\n// sixth\n// before\n"
    (repo / "src/patch.rs").write_text(patch_lines, encoding="utf-8")
    git(repo, "add", "src")
    git(repo, "commit", "-qm", "Write the judging base")
    base = git(repo, "rev-parse", "HEAD")

    (repo / "src/checkpoint.rs").write_text(original, encoding="utf-8")
    (repo / "src/base.rs").write_text("// lead\n// removed\n", encoding="utf-8")
    (repo / "src/stable.rs").write_text(original.replace("// tail", "// revised tail"), encoding="utf-8")
    (repo / "src/moved.rs").write_text(f"{TARGET}\n{TARGET}\n{original}", encoding="utf-8")
    (repo / "src/invalid.rs").write_bytes(original.encode() + b"// \xff\n")
    patch_text = patch_lines.replace("// before", "// after\v@@ -1,0 +1,1 @@\v+inserted")
    (repo / "src/patch.rs").write_text(patch_text, encoding="utf-8")
    git(repo, "add", "src")
    git(repo, "commit", "-qm", "Write the judging checkpoint")
    return base, git(repo, "rev-parse", "HEAD")


def commit_with_undecodable_path(repo: Path, checkpoint: str) -> str:
    """Add a raw path to a tree without asking the filesystem to store it."""
    entries = git(repo, "ls-tree", checkpoint).encode() + b"\n"
    blob = git(repo, "rev-parse", f"{checkpoint}:src/stable.rs")
    entry = b"100644 blob " + blob.encode() + b"\tinvalid-\xff.rs\n"
    tree = subprocess.run(
        ("git", "-C", str(repo), "mktree"), input=entries + entry, capture_output=True, check=True,
    ).stdout.decode().strip()
    return git(repo, "commit-tree", tree, "-p", checkpoint, "-m", "Add an undecodable path")


def seed_findings(
    database: Path, base: str, checkpoint: str, undecodable: str, cases: dict[str, FindingCase],
) -> None:
    """Insert stored histories after Ledger applies the real migrations."""
    with closing(sqlite3.connect(database)) as connection, connection:
        connection.execute("INSERT INTO sync (id, tip, synced_at) VALUES (1, ?, ?)", (checkpoint, STAMP))
        connection.executemany(
            """INSERT INTO tickets
            (id, unit, kind, family, rule, crate, files, sites, rules, rule_blobs, state, branch,
             checkpoint, created_at, updated_at, base)
             VALUES (?, 'unit', 'read', 'names', ?, '.', '[]', '[]', '[]', '{}',
                    'merged', 'main', ?, ?, ?, ?)""",
            [(identity, RULE, end, STAMP, STAMP, start) for identity, end, start in (
                ("judging", checkpoint, base), ("undecodable", undecodable, ""), ("empty", "", ""),
            )],
        )
        connection.executemany(
            """INSERT INTO held_findings
            (finding_id, rule, path, line, text, kind, before, after, rule_blob, ticket, state, proposal,
             created_at, updated_at, anchor_commit, followed_from, followed_from_line, followed_at, followed_from_commit)
            VALUES (?, ?, ?, ?, ?, 'rename', 'old_name', 'new_name', ?, ?, 'unfiled', '',
                    ?, ?, ?, ?, ?, ?, ?)""",
            (
                (identity, RULE, case.path, case.line, case.text, "1" * 40, case.ticket, STAMP, STAMP,
                 case.anchor, case.path if case.follow else "", case.line if case.follow else 0,
                 case.follow, base if case.follow else "")
                for identity, case in cases.items()
            ),
        )


def anchor_answers(repo: Path, database: Path, cases: dict[str, FindingCase], requested: list[str]) -> list[dict[str, object]]:
    """Read each requested id in order through Python's finding_anchor."""
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        findings = {finding.finding_id: finding for finding in ledger.held_findings(AllHeldFindings())}
        answers = []
        for identity in requested:
            if identity not in findings:
                answers.append({"case": "unknown", "answer": {"id": identity, "status": "unknown_finding"}})
                continue
            anchor = ledger.finding_anchor(findings[identity])
            if isinstance(anchor, FindingAnchor):
                answer = {"id": identity, "status": "anchored", "commit": anchor.commit,
                          "path": anchor.path, "line": anchor.line}
            else:
                answer = {"id": identity, "status": "needs_judgment", "reason": anchor.reason}
            answer["footprint"] = {"files": [findings[identity].path], "uncertain": []}
            answers.append({"case": cases[identity].name, "answer": answer})
        return answers


def ledger_variants(root: Path, ledger: Path, invalid_id: str) -> None:
    """Give each ledger failure and the revision row its own fixture."""
    for name in ("unsupported", "invalid", "invalid-path", "revision", "wal"):
        folder = root / "wal" if name == "wal" else root
        folder.mkdir(exist_ok=True)
        copy = folder / f"{name}.sqlite"
        shutil.copyfile(ledger, copy)
        with closing(sqlite3.connect(copy)) as connection, connection:
            if name == "unsupported":
                connection.execute("UPDATE schema_version SET version = ?", (max(MIGRATIONS) + 1,))
            elif name == "invalid":
                connection.execute("UPDATE held_findings SET line = 'invalid line' WHERE finding_id = ?", (invalid_id,))
            elif name == "invalid-path":
                connection.execute("UPDATE held_findings SET path = '../lib.rs', anchor_commit = (SELECT tip FROM sync) "
                                   "WHERE finding_id = ?", (invalid_id,))
            elif name == "revision":
                connection.execute("CREATE TABLE ledger_revision (revision INTEGER NOT NULL)")
                connection.execute("INSERT INTO ledger_revision VALUES (?)", (REVISION,))
            else:
                connection.execute("PRAGMA journal_mode = WAL").fetchone()


def preserve_wal(root: Path, ledger: Path) -> None:
    """Copy a committed WAL without copying the shared-memory file."""
    folder = root / "orphan"
    folder.mkdir()
    writer = root / "wal-writer.sqlite"
    shutil.copyfile(ledger, writer)
    with closing(sqlite3.connect(writer)) as connection:
        connection.execute("PRAGMA journal_mode = WAL").fetchone()
        with connection:
            connection.execute("CREATE TABLE ledger_revision (revision INTEGER NOT NULL)")
            connection.execute("INSERT INTO ledger_revision VALUES (?)", (REVISION,))
        for suffix in ("", "-wal"):
            shutil.copyfile(f"{writer}{suffix}", folder / f"wal.sqlite{suffix}")


def main(root: Path) -> None:
    """Print the ordered oracle answers and fixture metadata."""
    repo = root / "repo"
    base, checkpoint = source_commits(repo)
    undecodable = commit_with_undecodable_path(repo, checkpoint)
    rows = [
        FindingCase("followed", "src/stable.rs", anchor=base, follow=checkpoint),
        FindingCase("missing follow", "src/stable.rs", anchor=base, follow=MISSING_COMMIT),
        FindingCase("recorded", "src/stable.rs", text="recorded text", anchor=checkpoint),
        FindingCase("missing recorded anchor", "src/stable.rs", anchor=MISSING_COMMIT),
        FindingCase("no judging ticket", "src/stable.rs", ticket="absent"),
        FindingCase("same blob", "src/same.rs", text="descriptive legacy text"),
        FindingCase("checkpoint only", "src/checkpoint.rs"),
        FindingCase("base only", "src/base.rs"),
        FindingCase("unchanged line", "src/stable.rs"),
        FindingCase("moved line", "src/moved.rs"),
        FindingCase("no source match", "src/stable.rs", text="absent at both commits"),
        FindingCase("skip undecodable checkpoint", "src/invalid.rs"),
        FindingCase("vertical tab in patch header", "src/patch.rs", line=3),
        FindingCase("skip undecodable tree", "src/stable.rs", ticket="undecodable"),
        FindingCase("judging ticket without revisions", "src/stable.rs", ticket="empty"),
    ]
    cases = {f"{index:016x}": case for index, case in enumerate(rows, 1)}
    ledger = root / "ledger #1.sqlite"
    with Ledger(repo, ledger):
        pass
    seed_findings(ledger, base, checkpoint, undecodable, cases)
    identities = list(reversed(cases))
    requested = [identities[0], UNKNOWN_FINDING, *identities[1:], identities[0]]
    invalid_id = identities[0]
    answers = anchor_answers(repo, ledger, cases, requested)
    ledger_variants(root, ledger, invalid_id)
    preserve_wal(root, root / "wal/wal.sqlite")
    print(json.dumps({"synced_tip": checkpoint, "revision": REVISION, "invalid_id": invalid_id, "cases": answers},
                     sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
