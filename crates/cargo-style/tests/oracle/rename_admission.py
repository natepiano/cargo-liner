"""Build a ledger and read its rename admissions through the pinned Python API."""

from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
from contextlib import closing
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path

from scripts.style import approved_renames as renames
from scripts.style.style_ledger import AllHeldFindings, FindingAnchor, Ledger, LedgerAccess


RULE = "global:rust/rename-admission.md"
STAMP = "2026-01-01T00:00:00+00:00"
UNKNOWN = "f" * 16
MISSING_COMMIT = "f" * 40
PREVIOUS_PATH = "src/previous.rs"
LOCAL = "// π before the declaration\nfn sample() {\n    let g = 1; // café\n    consume(g);\n}\n"
CLOSURE = "fn sample() {\n    map(|g| g + 1);\n}\n"
CAPTURE = 'fn sample() {\n    let g = 1;\n    println!("{g}");\n}\n'
ATTRIBUTED_LOCAL = "attributed local"
ATTRIBUTE_POLICY = 'inert_attributes = ["allow"]\nstd_macros = []\n'
INVISIBLE_POLICY_FOLDER = "absent-\u200b"
POLICY_UNREAD = "rename presence policy unread: "


class History(Enum):
    """The stored evidence that chooses the finding's anchor."""

    BASE = "base"
    FOLLOW = "follow"
    SYNCED = "synced"
    LEGACY = "legacy"
    MISSING_FOLLOW = "missing follow"
    MISSING_ANCHOR = "missing anchor"
    NO_TICKET = "no ticket"


class CurrentSource(Enum):
    """The synced tree entry when no new source text is supplied."""

    UNCHANGED = "unchanged"
    MISSING = "missing"
    FOLDER = "folder"


@dataclass(frozen=True)
class AdmissionScenario:
    """One source history and the admission branch it must reach."""

    name: str
    original: str | bytes = LOCAL
    current: str | bytes | CurrentSource = CurrentSource.UNCHANGED
    line: int = 3
    history: History = History.BASE
    kind: str = "rename"
    before: str = "g"
    after: str = "wanted"
    recorded_text: str = ""
    expected: str = "admitted"

    @property
    def path(self) -> str:
        return f"src/{self.name.replace(' ', '-')}.rs"

    @property
    def text(self) -> str:
        source = self.original.decode(errors="replace") if isinstance(self.original, bytes) else self.original
        return self.recorded_text or source.splitlines()[self.line - 1]


def source_cases() -> tuple[AdmissionScenario, ...]:
    """Give each admission path independent source bytes."""
    return (
        AdmissionScenario("unchanged local"),
        AdmissionScenario("line moved", current="// A new line.\n" + LOCAL),
        AdmissionScenario("applied local", current=LOCAL.replace("g = ", "wanted = ").replace("(g)", "(wanted)")),
        AdmissionScenario("format capture", CAPTURE, CAPTURE.replace("g =", "wanted =").replace("{g}", "{wanted}"), line=2),
        AdmissionScenario("follow beats recorded anchor", history=History.FOLLOW),
        AdmissionScenario("legacy same blob", history=History.LEGACY),
        AdmissionScenario("legacy checkpoint only", LOCAL.replace("g = 1", "g = 2"), LOCAL,
             history=History.LEGACY, recorded_text=LOCAL.splitlines()[2]),
        AdmissionScenario("changed signature", CLOSURE, CLOSURE.replace("sample()", "sample(extra: usize)"), line=2),
        AdmissionScenario("invalid UTF8 current source", current=LOCAL.encode().replace(b"caf\xc3\xa9", b"caf\xff")),
        AdmissionScenario("inert attribute and standard macro", "#[allow(dead_code)]\n" + CAPTURE, line=3),
        AdmissionScenario(ATTRIBUTED_LOCAL, "#[allow(dead_code)]\n" + LOCAL, line=4),
        AdmissionScenario("qualified rename", before="Parent::g", after="Parent::wanted", history=History.NO_TICKET,
             expected="non_local_reference_companion"),
        AdmissionScenario("item declaration", "fn g() {}\n", line=1, expected="non_local_reference_companion"),
        AdmissionScenario("field declaration", "struct Record {\n    g: usize,\n}\n", line=2,
             expected="non_local_reference_companion"),
        AdmissionScenario("variant declaration", "enum Choice {\n    g,\n}\n", line=2, expected="non_local_reference_companion"),
        AdmissionScenario("descriptive reference", "fn describe() { consume(17); }\n", line=1,
             expected="non_local_reference_companion"),
        AdmissionScenario("non rename operation", kind="restructure", history=History.NO_TICKET,
             expected="non_local_reference_companion"),
        AdmissionScenario("invalid identifier pair", before="g + e", after="wanted + other", history=History.NO_TICKET,
             expected="non_local_reference_companion"),
        AdmissionScenario("legacy ambiguous coordinate", current="// New first line.\n// New second line.\n" +
             LOCAL.splitlines()[2] + "\n" + LOCAL, history=History.LEGACY,
             expected="source anchor is ambiguous between base and checkpoint"),
        AdmissionScenario("no judging ticket", history=History.NO_TICKET, expected="no judging ticket for source anchor"),
        AdmissionScenario("missing recorded anchor", history=History.MISSING_ANCHOR, expected="recorded anchor commit is missing"),
        AdmissionScenario("missing last follow", history=History.MISSING_FOLLOW, expected="last follow commit is missing"),
        AdmissionScenario("stale recorded text", recorded_text="let g = 4;", expected="finding text is absent"),
        AdmissionScenario("missing current source", current=CurrentSource.MISSING, expected="current source file is missing"),
        AdmissionScenario("missing anchor file", current=CurrentSource.MISSING, history=History.SYNCED,
             expected="source anchor file is missing"),
        AdmissionScenario("folder at current path", current=CurrentSource.FOLDER, expected="current source file is missing"),
        AdmissionScenario("invalid UTF8 anchor", original=LOCAL.encode().replace(b"caf\xc3\xa9", b"caf\xff"),
             expected="source anchor file is not UTF-8"),
        AdmissionScenario("ambiguous declaration", "fn sample() {\n    let g = 1; let g = 2;\n}\n", line=2,
             expected="finding line has no unique declaration of 'g'"),
        AdmissionScenario("changed item identity", current=LOCAL.replace("fn sample()", "fn replacement()"),
             expected="recorded item path is not unique in current source"),
        AdmissionScenario("repeated item identity", current=LOCAL + LOCAL,
             expected="recorded item path is not unique in current source"),
        AdmissionScenario("unread body", "fn sample() { let task = async move { 1 };\n    map(|g| g + 1);\n}\n", line=2,
             expected="containing item body unread"),
        AdmissionScenario("unread header", "fn sample(value: impl Trait { fn hidden() {} }) {\n    map(|g| g + 1);\n}\n",
             line=2, expected="containing item body unread"),
        AdmissionScenario("unread item structure", CLOSURE, CLOSURE.rstrip()[:-1], line=2,
             expected="recorded containing item unread"),
        AdmissionScenario("unsupported shorthand", "fn sample() {\n    let g = 1;\n    consume(Value { g });\n}\n",
             "fn sample() {\n    let wanted = 1;\n    consume(Value { g: wanted });\n}\n", line=2,
             expected="current rename's own edits cannot be identified exactly"),
        AdmissionScenario("unread current tokens", current=LOCAL + "/* unfinished\n", expected="target tokens unread"),
        AdmissionScenario("incomplete alignment", current=LOCAL.replace("consume(g)", "discard(g)"),
             expected="current rename's own edits cannot be identified exactly"),
        AdmissionScenario("unread binding visibility", "fn sample() {\n    let g = 1;\n    unknown!(let g = 2);\n}\n", line=2,
             expected="current rename edits have unread binding visibility"),
        AdmissionScenario("outer binding", CLOSURE, CLOSURE.replace("sample()", "sample(wanted: usize)"), line=2,
             expected="rename blocked by outer binding 'wanted' at line 1"),
        AdmissionScenario("wanted name used", "fn sample() {\n    let g = 1;\n    consume(wanted);\n}\n", line=2,
             expected="wanted name already used in this item: 'wanted' at line 3"),
        AdmissionScenario("unknown bang macro", "fn sample() {\n    let g = 1;\n    unknown!(g);\n}\n", line=2,
             expected="unresolved bang macro 'unknown' at line 3"),
        AdmissionScenario("shadowed standard macro", 'macro_rules! println { () => {} }\n' + CAPTURE, line=3,
             expected="unresolved bang macro 'println' at line 4"),
        AdmissionScenario("inherited inner attribute", "mod container {\n    #![inject]\n" + CLOSURE + "}\n", line=4,
             expected="unresolved attribute macro 'inject' at line 2"),
        AdmissionScenario("qualified attribute", "#[module::allow]\n" + CLOSURE, line=3,
             expected="unresolved attribute macro 'module' at line 1"),
    )


def git(repo: Path, *arguments: str) -> str:
    """Run Git with isolated configuration."""
    return subprocess.run(
        ("git", "-C", str(repo), *arguments), capture_output=True, check=True, text=True,
    ).stdout.strip()


def write_source(path: Path, source: str | bytes) -> None:
    path.write_bytes(source.encode() if isinstance(source, str) else source)


def source_commits(repo: Path, rows: dict[str, AdmissionScenario]) -> tuple[str, str]:
    """Commit every recorded file and each change at the synced tip."""
    git(repo, "config", "user.name", "Rename Admission Test")
    git(repo, "config", "user.email", "rename@example.invalid")
    git(repo, "config", "commit.gpgsign", "false")
    git(repo, "config", "core.hooksPath", "/dev/null")
    git(repo, "config", "maintenance.auto", "false")
    (repo / "src").mkdir()
    for row in rows.values():
        path = PREVIOUS_PATH if row.history is History.FOLLOW else row.path
        write_source(repo / path, row.original)
    git(repo, "add", "src")
    git(repo, "commit", "-qm", "Record the judged source")
    base = git(repo, "rev-parse", "HEAD")
    for row in rows.values():
        if row.history is History.FOLLOW:
            (repo / PREVIOUS_PATH).rename(repo / row.path)
        if row.current is CurrentSource.UNCHANGED:
            continue
        path = repo / row.path
        if isinstance(row.current, CurrentSource):
            path.unlink()
            if row.current is CurrentSource.FOLDER:
                path.mkdir()
                (path / "child.rs").write_text("fn child() {}\n", encoding="utf-8")
        else:
            write_source(path, row.current)
    git(repo, "add", "src")
    git(repo, "commit", "-qm", "Sync the current source")
    return base, git(repo, "rev-parse", "HEAD")


def seed_findings(database: Path, base: str, tip: str, rows: dict[str, AdmissionScenario]) -> None:
    """Store each finding with its chosen source history."""
    with closing(sqlite3.connect(database)) as connection, connection:
        connection.execute("INSERT INTO sync (id, tip, synced_at) VALUES (1, ?, ?)", (tip, STAMP))
        connection.execute(
            """INSERT INTO tickets
            (id, unit, kind, family, rule, crate, files, sites, rules, rule_blobs, state, branch,
             checkpoint, created_at, updated_at, base)
             VALUES ('judging', 'unit', 'read', 'names', ?, '.', '[]', '[]', '[]', '{}',
                     'merged', 'main', ?, ?, ?, ?)""", (RULE, tip, STAMP, STAMP, base),
        )
        for identity, row in rows.items():
            anchor = {History.BASE: base, History.SYNCED: tip, History.FOLLOW: MISSING_COMMIT,
                      History.MISSING_ANCHOR: MISSING_COMMIT}.get(row.history, "")
            follow = {History.FOLLOW: tip, History.MISSING_FOLLOW: MISSING_COMMIT}.get(row.history, "")
            ticket = "absent" if row.history is History.NO_TICKET else "judging"
            connection.execute(
                """INSERT INTO held_findings
                (finding_id, rule, path, line, text, kind, before, after, rule_blob, ticket, state, proposal,
                 created_at, updated_at, anchor_commit, followed_from, followed_from_line, followed_at, followed_from_commit)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'unfiled', '', ?, ?, ?, ?, ?, ?, ?)""",
                (identity, RULE, row.path, row.line, row.text, row.kind, row.before, row.after, "1" * 40, ticket,
                 STAMP, STAMP, anchor, PREVIOUS_PATH if follow else "", 8 if follow else 0, follow,
                 base if follow else ""),
            )


def admission_fields(result: renames.RenameAdmission) -> dict[str, object]:
    """Project the Python result to the public reply fields."""
    if isinstance(result, renames.AdmittedLocalRename):
        fields = asdict(result)
        fields["anchor"] = {"commit": result.anchor.commit, "path": result.anchor.path, "line": result.anchor.line}
        return {"status": "admitted", **fields}
    if isinstance(result, renames.NonLocalReferenceCompanion):
        return {"status": "non_local_reference_companion"}
    if isinstance(result, renames.OuterBindingBlocked):
        return {"status": "blocked", **asdict(result)}
    return {"status": "unproved", **asdict(result)}


def read_answers(
    ledger: Ledger, tip: str, rows: dict[str, AdmissionScenario], requested: list[str], policy: Path,
    policy_failure: str = "", *, case_name: str = "",
) -> list[dict[str, object]]:
    """Call the ledger for each ID with the named fixture policy."""
    findings = {finding.finding_id: finding for finding in ledger.held_findings(AllHeldFindings())}
    answers: list[dict[str, object]] = []
    saved_module_path = renames.__file__
    renames.__file__ = str(policy.with_name("approved_renames.py"))
    try:
        for identity in requested:
            if identity not in rows:
                answers.append({"name": "unknown", "answer": {"id": identity, "status": "unknown_finding"}})
                continue
            row, finding = rows[identity], findings[identity]
            anchor = ledger.finding_anchor(finding)
            anchor_fields = ({"status": "anchored", **asdict(anchor)} if isinstance(anchor, FindingAnchor)
                             else {"status": "needs_judgment", "reason": anchor.reason})
            blob = ledger._commit_file(tip, finding.path)
            source = (blob.decode(errors="replace") if isinstance(blob, bytes)
                      else renames.CurrentRenameSourceUnavailable())
            admission = admission_fields(ledger.local_rename_admission(finding, finding.path, source))
            observed = admission.get("reason", admission["status"])
            if policy_failure:
                assert isinstance(observed, str) and observed.startswith(policy_failure), (row.name, admission)
            else:
                assert observed == row.expected, (row.name, admission, row.expected)
            answers.append({"name": case_name or row.name,
                            "answer": {"id": identity, **anchor_fields, "rename_admission": admission}})
    finally:
        renames.__file__ = saved_module_path
    return answers


def main(root: Path) -> None:
    """Write policy fixtures and print the expected command replies."""
    repo, database = root / "repo", root / "ledger.sqlite"
    rows = {f"{index:016x}": row for index, row in enumerate(source_cases(), 1)}
    base, tip = source_commits(repo, rows)
    with Ledger(repo, database):
        pass
    seed_findings(database, base, tip, rows)
    policies = {name: root / name / "rename_policy.toml" for name in ("valid", "malformed", "absent")}
    for policy in policies.values():
        policy.parent.mkdir()
    policies["valid"].write_text('inert_attributes = ["allow"]\nstd_macros = ["println"]\n', encoding="utf-8")
    policies["malformed"].write_text('inert_attributes = false\nstd_macros = ["println"]\n', encoding="utf-8")
    local = next(iter(rows))
    attributed = next(identity for identity, row in rows.items() if row.name == ATTRIBUTED_LOCAL)
    early = next(identity for identity, row in rows.items() if row.name == "qualified rename")
    ordered = list(reversed(rows))
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        requests = [
            {"policy": "omitted", "cases": read_answers(ledger, tip, rows, [early, UNKNOWN], policies["absent"])},
            {"policy": "valid", "cases": read_answers(ledger, tip, rows, [ordered[0], UNKNOWN, *ordered[1:], ordered[0]],
                                                      policies["valid"])},
            {"policy": "malformed", "cases": read_answers(ledger, tip, rows, [local], policies["malformed"],
                                                           "rename presence policy unread: inert_attributes must be a list of names")},
            {"policy": "absent", "cases": read_answers(ledger, tip, rows, [local], policies["absent"],
                                                       "rename presence policy unread: [Errno 2]")},
        ]
        policy_rows = (
            ("hex_escape", "hex escape in attribute policy", r'inert_attributes = ["\x61llow"]' + '\nstd_macros = []\n',
             "Unescaped '\\' in a string"),
            ("inline_table_trailing_comma", "trailing comma in inline policy table", ATTRIBUTE_POLICY + "extra = { x = 1, }\n",
             "Invalid initial character for a key part"),
            ("duplicate_table", "duplicate policy table", ATTRIBUTE_POLICY + "[extra]\n[extra]\n",
             "Cannot declare ('extra',) twice (at line 4, column 7)"),
        )
        for fixture, name, text, reason in policy_rows:
            policy = root / fixture / "rename_policy.toml"
            policy.parent.mkdir()
            policy.write_text(text, encoding="utf-8")
            requests.append({"policy": fixture, "cases": read_answers(
                ledger, tip, rows, [attributed], policy, POLICY_UNREAD + reason, case_name=name)})
        missing_policy = root / INVISIBLE_POLICY_FOLDER / "rename_policy.toml"
        missing_policy.parent.mkdir()
        missing_reason = f"[Errno 2] No such file or directory: {str(missing_policy)!r}"
        assert r"\u200b" in missing_reason, missing_reason
        requests.append({"policy": "absent_zero_width_space", "cases": read_answers(
            ledger, tip, rows, [attributed], missing_policy, POLICY_UNREAD + missing_reason,
            case_name="missing policy in folder with zero-width space")})
    print(json.dumps({"synced_tip": tip, "policy_required": local, "requests": requests}, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
