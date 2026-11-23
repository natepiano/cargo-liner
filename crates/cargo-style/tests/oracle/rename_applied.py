"""Read rename results from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
from contextlib import closing
from dataclasses import asdict, dataclass, replace
from enum import Enum
from pathlib import Path

from scripts.style import approved_renames as renames
from scripts.style.style_ledger import Ledger, LedgerAccess, LedgerError


RULE = "global:rust/rename-applied.md"
RULES = json.dumps([RULE])
STAMP = "2026-01-01T00:00:00+00:00"
UNKNOWN = "f" * 16
LOCAL = "fn f() {\n    let old = input;\n    consume(old);\n}\n"
EDITED = LOCAL.replace("old", "fresh")
MISSING_BLOB_SOURCE = "// Unique source for the failed read.\n" + LOCAL
MISSING_BLOB_PATH = "src/failed-read.rs"


class Revision(Enum):
    """The revision a ticket records."""

    NONE = "no revision"
    EMPTY = "before the source files"
    BASE = "original source"
    CHECKPOINT = "previous checkpoint"
    TIP = "synced tip"


class AbsentSource(Enum):
    """The file is absent at this revision."""

    ABSENT = "absent"


@dataclass(frozen=True)
class Finding:
    """The stored site and pair for one case."""

    name: str
    path: str
    text: str
    expect: str
    line: int = 2
    before: str = "old"
    after: str = "fresh"
    kind: str = "rename"
    build_ticket: str = ""
    state: str = "unfiled"
    anchor: Revision = Revision.BASE


@dataclass(frozen=True)
class Ticket:
    """The exact ticket fields used to select proof members."""

    id: str
    members: tuple[str, ...] = ()
    state: str = "merged"
    base: Revision = Revision.BASE
    checkpoint: Revision = Revision.TIP
    branch: str = "work"
    rules: str = RULES
    batch: str = ""


class Cases:
    """Sources and saved evidence for all requests."""

    def __init__(self) -> None:
        self.base: dict[str, str | bytes] = {}
        self.tip: dict[str, str | bytes] = {}
        self.findings: dict[str, Finding] = {}
        self.tickets: list[Ticket] = []

    def add(self, finding: Finding) -> str:
        identity = f"{len(self.findings) + 1:016x}"
        self.findings[identity] = finding
        return identity

    def source(self, path: str, original: str | bytes | AbsentSource, edited: str | bytes | AbsentSource) -> None:
        if isinstance(original, str | bytes):
            self.base[path] = original
        if isinstance(edited, str | bytes):
            self.tip[path] = edited

    def binding(self, name: str, original: str | bytes | AbsentSource = LOCAL,
                edited: str | bytes | AbsentSource = EDITED,
                *, expect: str = "applied_in_code", line: int = 2, text: str = "",
                before: str = "old", after: str = "fresh", state: str = "unfiled") -> None:
        path = f"src/{name.replace(' ', '-')}.rs"
        self.source(path, original, edited)
        source = original.decode(errors="replace") if isinstance(original, bytes) else original
        if isinstance(source, AbsentSource):
            source = LOCAL
        ticket = f"direct-{len(self.findings):03d}"
        identity = self.add(Finding(name, path, text or source.splitlines()[line - 1], expect,
                                    line, before, after, build_ticket=ticket, state=state))
        self.tickets.append(Ticket(ticket, (identity,)))


def binding_rows(cases: Cases) -> None:
    """Vary the source evidence for a single local edit."""
    cases.binding("applied snippet", before="(old, old)", after="(fresh, fresh)")
    cases.binding("unchanged", edited=LOCAL, expect="not_applied")
    cases.binding("partial and missed use", edited=LOCAL.replace("let old", "let fresh"), expect="no_applied_check")
    cases.binding("unrelated token edit", edited=EDITED.replace("input", "value"), expect="unproved")
    shadow = LOCAL.replace("    consume", "    { let old = inner; consume(old); }\n    consume")
    cases.binding("shadowed binding", shadow, shadow.replace("let old = input", "let fresh = input")
                  .replace("    consume(old);", "    consume(fresh);"))
    shorthand = LOCAL.replace("consume(old)", "consume(Value { old })")
    cases.binding("shorthand expansion", shorthand, shorthand.replace("let old", "let fresh")
                  .replace("{ old }", "{ old: fresh }"), expect="unproved")
    collapse = LOCAL.replace("consume(old)", "consume(Value { fresh: old })")
    cases.binding("shorthand collapse", collapse, collapse.replace("let old", "let fresh")
                  .replace("fresh: old", "fresh"), expect="unproved")
    capture = LOCAL.replace("consume(old)", 'println!("{old}")')
    cases.binding("format capture", capture, capture.replace("old", "fresh"))
    cases.binding("encoded capture", capture.replace("{old}", r"\x7bold}"),
                  capture.replace("{old}", r"\x7bfresh}").replace("let old", "let fresh"), expect="unproved")
    commented = LOCAL.replace("consume(old);", "consume(old); // Keep `old`")
    cases.binding("comment edit", commented, commented.replace("old", "fresh"))
    cases.binding("missing comment edit", commented, commented.replace("let old", "let fresh")
                  .replace("(old)", "(fresh)"), expect="no_applied_check")
    cases.binding("unexplained binding comment", commented, commented.replace("old", "fresh").replace("Keep", "Use"),
                  expect="no_applied_check")
    cases.binding("applied with unexplained file edit", "// Before\n" + LOCAL, "// After\n" + EDITED, line=3)
    cases.binding("absent finding text", text="let old = absent;", expect="unproved")
    tied = "fn f() {\n    let old = input;\n} fn g() {\n    let old = input;\n}\n"
    cases.binding("tied finding text", tied, tied.replace("old", "fresh"), line=3,
                  text="let old = input;", expect="unproved")
    cases.binding("nonbinding source line", line=3, expect="unproved")
    cases.binding("glob import", "use provider::*;\n" + LOCAL, "use provider::*;\n" + EDITED,
                  line=3, expect="no_applied_check")
    cases.binding("reused new name", LOCAL.replace("consume(old)", "consume(old, fresh)"), EDITED,
                  expect="unproved")
    cases.binding("old name is a type", "struct old;\n" + LOCAL, "struct old;\n" + EDITED,
                  line=3, expect="no_applied_check")
    cases.binding("raw pair", before="r#old", expect="no_applied_check")
    parameter = "fn f(old: T, fresh: T) { consume(old); }\n"
    cases.binding("parameter collision", parameter, parameter.replace("old", "fresh"), line=1, expect="unproved")
    capture_collision = 'fn f() {\n    let old = input;\n    println!("{old} {fresh}");\n}\n'
    cases.binding("capture collision", capture_collision, capture_collision.replace("old", "fresh"),
                  expect="no_applied_check")
    uncertain = "use provider::println;\n" + capture
    cases.binding("uncertain macro origin", uncertain, uncertain.replace("old", "fresh"), line=3,
                  expect="unproved")
    cases.binding("blocked override", LOCAL.replace("f()", "f(fresh: T)"), EDITED.replace("f()", "f(fresh: T)"),
                  expect="blocked")
    attribute = "#[inject]\n" + LOCAL
    cases.binding("unproved override after edit proof", attribute, attribute.replace("old", "fresh"),
                  line=3, expect="unproved")
    cases.binding("base path missing", AbsentSource.ABSENT, EDITED, expect="unproved")
    cases.binding("target path missing", LOCAL, AbsentSource.ABSENT, expect="unproved")
    cases.binding("replacement decoding", b"// \xff\n" + LOCAL.encode(), b"// \xff\n" + EDITED.encode(), line=3)


def ticket_rows(cases: Cases) -> None:
    """Change one condition at a time in the ticket selection."""
    for name, state, checkpoint, expect in (
        ("direct assigned", "assigned", Revision.NONE, "applied_in_code"),
        ("direct checkpointed at ancestor", "checkpointed", Revision.CHECKPOINT, "applied_in_code"),
        ("direct merged and finding built", "merged", Revision.TIP, "applied_in_code"),
        ("released at exact checkpoint", "merged", Revision.TIP, "applied_in_code"),
        ("released at descendant", "merged", Revision.CHECKPOINT, "no_applied_check"),
        ("released but checkpointed", "checkpointed", Revision.TIP, "no_applied_check"),
        ("released but assigned", "assigned", Revision.TIP, "no_applied_check"),
    ):
        path = f"src/{name.replace(' ', '-')}.rs"
        cases.source(path, LOCAL, EDITED)
        ticket = name
        build = ticket if name.startswith("direct") else ""
        finding_state = "built" if name.endswith("finding built") else "unfiled"
        identity = cases.add(Finding(name, path, LOCAL.splitlines()[1], expect, build_ticket=build,
                                     state=finding_state))
        cases.tickets.append(Ticket(ticket, (identity,), state=state, checkpoint=checkpoint))
    for name, first_base, second_base, direct, member, expect in (
        ("first eligible linked ticket wins", Revision.EMPTY, Revision.BASE, False, True, "unproved"),
        ("skip linked empty base", Revision.NONE, Revision.BASE, False, True, "applied_in_code"),
        ("direct ticket beats earlier linked ticket", Revision.EMPTY, Revision.BASE, True, True, "applied_in_code"),
        ("direct empty base ignores linked ticket", Revision.BASE, Revision.NONE, True, True, "no_applied_check"),
        ("direct missing member ignores linked ticket", Revision.BASE, Revision.BASE, True, False, "no_applied_check"),
    ):
        path = f"src/{name.replace(' ', '-')}.rs"
        cases.source(path, LOCAL, EDITED)
        first, second = f"{name}-1", f"{name}-2"
        direct_id = second if direct else ""
        identity = cases.add(Finding(name, path, LOCAL.splitlines()[1], expect, build_ticket=direct_id))
        cases.tickets.extend((Ticket(second, (identity,) if member else (), base=second_base),
                              Ticket(first, (identity,), base=first_base)))


def companion_rows(cases: Cases) -> None:
    """Prove the scoped pair with the exact ticket membership."""
    source = "fn f(old: T, other: T) { consume(old); consume(other); }\n"
    edited = "\n" + source.replace("old", "fresh").replace("other", "second")
    path = "src/companions.rs"
    cases.source(path, source, edited)
    first = cases.add(Finding("companion changes and moves the same line", path, source.strip(), "applied_in_code",
                              line=1, build_ticket="companions"))
    second = cases.add(Finding("second approved companion", path, source.strip(), "applied_in_code",
                               line=1, before="other", after="second", build_ticket="companions"))
    cases.tickets.append(Ticket("companions", (first, second)))
    for name, line, text, member_path in (
        ("member with different line", 3, LOCAL.splitlines()[1], "src/member-with-different-line.rs"),
        ("member with different text", 2, "let old = missing;", "src/member-with-different-text.rs"),
        ("member on different path", 2, LOCAL.splitlines()[1], "src/other-member-path.rs"),
    ):
        path = f"src/{name.replace(' ', '-')}.rs"
        cases.source(path, LOCAL, EDITED)
        cases.add(Finding(name, path, LOCAL.splitlines()[1], "no_applied_check", build_ticket=name))
        member = cases.add(Finding(f"{name} companion", member_path, text, "unproved", line=line))
        cases.tickets.append(Ticket(name, (member,)))


def batch_rows(cases: Cases) -> None:
    """Include a batch member only when its saved fields match."""
    for name, changes in (
        ("matching checkpointed batch member", {"state": "checkpointed"}),
        ("matching merged batch member", {}),
        ("batch member with other batch", {"batch": "different"}),
        ("batch member with wrong state", {"state": "assigned"}),
        ("batch member with wrong base", {"base": Revision.CHECKPOINT}),
        ("batch member with wrong branch", {"branch": "other"}),
        ("batch member with wrong checkpoint", {"checkpoint": Revision.CHECKPOINT}),
        ("batch member with wrong rules", {"rules": json.dumps(["other:rule"])}),
        ("batch member with different serialized rules", {"rules": json.dumps([RULE], indent=2)}),
        ("batch root compares a descendant", {}),
    ):
        path = f"src/{name.replace(' ', '-')}.rs"
        cases.source(path, LOCAL, EDITED)
        root = f"{name}-root"
        checkpoint = Revision.CHECKPOINT if name == "batch root compares a descendant" else Revision.TIP
        expect = "applied_in_code" if name.startswith("matching") else "no_applied_check"
        identity = cases.add(Finding(name, path, LOCAL.splitlines()[1], expect, build_ticket=root))
        cases.tickets.append(Ticket(root, checkpoint=checkpoint, batch=root))
        cases.tickets.append(replace(Ticket(f"{name}-member", (identity,), batch=root), **changes))


def file_rows(cases: Cases) -> None:
    """Vary the file and child module entries at the target."""
    for name, has_old, has_new, folder, extra, expect in (
        ("only old file", True, False, "", "", "not_applied"),
        ("only new file", False, True, "", "", "applied_in_code"),
        ("both files", True, True, "", "", "no_applied_check"),
        ("neither file", False, False, "", "", "no_applied_check"),
        ("old stem child module", False, True, "old", "", "no_applied_check"),
        ("new stem child module", False, True, "fresh", "", "no_applied_check"),
        ("instructions name another change", False, True, "", " Then move the code to helper.rs.", "no_applied_check"),
        ("instructions use dotted capital I", False, True, "", " Then SPLİT the code.", "no_applied_check"),
        ("instructions separate a second file pair with U+001F", False, True, "",
         " Then rename other.rs\x1fto helper.rs.", "no_applied_check"),
        ("instructions use a decomposed accent", False, True, "", " No e\u0301 definitions move.", "no_applied_check"),
    ):
        directory = f"src/{name.replace(' ', '-')}"
        path = f"{directory}/old.rs"
        cases.base[path] = LOCAL
        if has_old:
            cases.tip[path] = LOCAL
        if has_new:
            cases.tip[f"{directory}/fresh.rs"] = LOCAL
        if folder:
            cases.tip[f"{directory}/{folder}/child.rs"] = "fn child() {}\n"
        cases.add(Finding(name, path, LOCAL.splitlines()[1], expect, before="",
                          after=f"Rename old.rs to fresh.rs.{extra}", build_ticket="missing-ticket"))
    cases.base["src/old.rs"] = LOCAL
    cases.tip["src/fresh.rs"] = LOCAL
    cases.add(Finding("empty tree at old stem", "src/old.rs", LOCAL.splitlines()[1], "applied_in_code",
                      before="", after="Rename old.rs to fresh.rs."))


def qualified_rows(cases: Cases) -> None:
    """Ask for a declaration of exactly one spelling under the parent."""
    for name, parent, source, expect in (
        ("inherent method", "Parent", "struct Parent; impl Parent { fn fresh() {} }", "applied_in_code"),
        ("trait method", "Parent", "trait Parent { fn fresh(); }", "applied_in_code"),
        ("field", "Parent", "struct Parent { fresh: usize }", "applied_in_code"),
        ("enum variant", "Parent", "enum Parent { fresh }", "applied_in_code"),
        ("module item", "parent", "mod parent { fn fresh() {} }", "applied_in_code"),
        ("old method remains", "Parent", "struct Parent; impl Parent { fn old() {} }", "not_applied"),
        ("both names present", "Parent", "trait Parent { fn old(); fn fresh(); }", "no_applied_check"),
        ("neither name present", "Parent", "trait Parent { fn other(); }", "no_applied_check"),
        ("reference without declaration", "Parent", "fn f() { Parent::fresh(); }", "no_applied_check"),
        ("ambiguous parents", "Parent", "mod a { struct Parent { fresh: usize } } mod b { struct Parent; }",
         "no_applied_check"),
        ("ambiguous impl", "Parent", "struct Parent; impl Other::Parent { fn fresh() {} }", "no_applied_check"),
        ("unsafe derive", "Parent", "#[derive(Custom)] struct Parent { fresh: usize }", "no_applied_check"),
        ("name macro", "Parent", "paste! { struct Parent { fresh: usize } }", "no_applied_check"),
        ("token paste", "Parent", "macro_rules! m { () => { [< fresh >] } } struct Parent { fresh: usize }",
         "no_applied_check"),
        ("qualified source missing", "Parent", AbsentSource.ABSENT, "no_applied_check"),
        ("other finding kind", "Parent", "struct Parent { fresh: usize }", "no_applied_check"),
    ):
        path = f"src/{name.replace(' ', '-')}.rs"
        old = f"struct {parent} {{ old: usize }}\n"
        cases.source(path, old, source)
        kind = "type" if name == "other finding kind" else "rename"
        cases.add(Finding(name, path, old.strip(), expect, line=1, before=f"{parent}::old",
                          after=f"{parent}::fresh", kind=kind, build_ticket="missing-ticket"))


def git(repo: Path, *arguments: str, input_text: str = "") -> str:
    return subprocess.run(("git", "-C", str(repo), *arguments), capture_output=True,
                          input=input_text, check=True, text=True).stdout.strip()


def commit(repo: Path, name: str) -> str:
    git(repo, "add", "-A")
    git(repo, "commit", "-qm", name, "--allow-empty")
    return git(repo, "rev-parse", "HEAD")


def write_files(repo: Path, files: dict[str, str | bytes]) -> None:
    for path, text in files.items():
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(text.encode() if isinstance(text, str) else text)


def commit_empty_stem_tree(repo: Path, parent: str) -> str:
    """Add an empty tree that the worktree index cannot store."""
    empty = git(repo, "mktree")
    entries = git(repo, "ls-tree", f"{parent}:src")
    source = git(repo, "mktree", input_text=f"{entries}\n040000 tree {empty}\told\n")
    root = git(repo, "mktree", input_text=f"040000 tree {source}\tsrc\n")
    tip = git(repo, "commit-tree", root, "-p", parent, "-m", "Record an empty old stem tree")
    git(repo, "update-ref", "HEAD", tip, parent)
    return tip


def source_revisions(repo: Path, cases: Cases) -> dict[Revision, str]:
    for key, value in (("user.name", "Rename Applied Test"), ("user.email", "applied@example.invalid"),
                       ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null"), ("gc.auto", "0")):
        git(repo, "config", key, value)
    empty = commit(repo, "Create the repository")
    write_files(repo, cases.base)
    base = commit(repo, "Record source before the edits")
    checkpoint = commit(repo, "Record a separate checkpoint")
    for path in cases.base.keys() - cases.tip.keys():
        (repo / path).unlink()
    write_files(repo, cases.tip)
    tip = commit_empty_stem_tree(repo, commit(repo, "Sync the target source"))
    return {Revision.NONE: "", Revision.EMPTY: empty, Revision.BASE: base,
            Revision.CHECKPOINT: checkpoint, Revision.TIP: tip}


def seed_ledger(database: Path, revisions: dict[Revision, str], cases: Cases) -> None:
    with closing(sqlite3.connect(database)) as connection, connection:
        connection.execute("INSERT INTO sync (id, tip, synced_at) VALUES (1, ?, ?)", (revisions[Revision.TIP], STAMP))
        for ticket in (Ticket("judging"), *cases.tickets):
            connection.execute(
                """INSERT INTO tickets
                (id, unit, kind, family, rule, crate, files, sites, rules, rule_blobs, state, branch,
                 checkpoint, created_at, updated_at, base, finding_ids)
                 VALUES (?, ?, 'proposal', 'names', ?, '.', '[]', '[]', ?, '{}', ?, ?, ?, ?, ?, ?, ?)""",
                (ticket.id, ticket.id, RULE, ticket.rules, ticket.state, ticket.branch,
                 revisions[ticket.checkpoint], STAMP, STAMP, revisions[ticket.base], json.dumps(ticket.members)),
            )
            if ticket.batch:
                connection.execute("INSERT INTO bulk_workspace_tickets (ticket, batch) VALUES (?, ?)",
                                   (ticket.id, ticket.batch))
        for identity, row in cases.findings.items():
            connection.execute(
                """INSERT INTO held_findings
                (finding_id, rule, path, line, text, kind, before, after, rule_blob, ticket, state,
                 proposal, created_at, updated_at, anchor_commit, build_ticket)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'judging', ?, '', ?, ?, ?, ?)""",
                (identity, RULE, row.path, row.line, row.text, row.kind, row.before, row.after, "1" * 40,
                 row.state, STAMP, STAMP, revisions[row.anchor], row.build_ticket),
            )


def result_value(result: renames.AppliedCheck) -> dict[str, object]:
    if isinstance(result, renames.AppliedInCode):
        return {"status": "applied_in_code"}
    if isinstance(result, renames.RenameNotApplied):
        return {"status": "not_applied", **asdict(result)}
    if isinstance(result, renames.NoAppliedCheck):
        return {"status": "no_applied_check", **asdict(result)}
    if isinstance(result, renames.OuterBindingBlocked):
        return {"status": "blocked", **asdict(result)}
    return {"status": "unproved", **asdict(result)}


def assert_edit_proof(cases: Cases) -> None:
    """Check that the override and the whole-file result can change independently."""
    for name in ("applied with unexplained file edit", "unproved override after edit proof"):
        row = next(row for row in cases.findings.values() if row.name == name)
        pair = renames.binding_rename(row.kind, row.before, row.after, row.line, row.text)
        assert isinstance(pair, renames.BindingRename), row.name
        before, after = cases.base[row.path], cases.tip[row.path]
        assert isinstance(before, str) and isinstance(after, str), row.name
        proof = renames.binding_rename_check(row.path, before, after, (pair,))
        assert isinstance(proof.checks[pair], renames.AppliedInCode), (row.name, proof)
        assert proof.explained == (name == "unproved override after edit proof"), (row.name, proof)


def answers(ledger: Ledger, tip: str, cases: Cases, ids: list[str]) -> list[dict[str, object]]:
    readings: list[dict[str, object]] = []
    for identity in ids:
        if identity == UNKNOWN:
            readings.append({"name": "unknown finding", "id": UNKNOWN, "result": {"status": "unknown_finding"}})
            continue
        row = cases.findings[identity]
        result = result_value(ledger.applied_check(ledger._held(identity), tip))
        assert result["status"] == row.expect, (row.name, result, row.expect)
        readings.append({"name": row.name, "id": identity, "result": result})
    return readings


def python_failure(ledger: Ledger, tip: str, identity: str, expected: str) -> str:
    try:
        ledger.applied_check(ledger._held(identity), tip)
    except LedgerError as error:
        assert expected in str(error), str(error)
        return str(error)
    raise AssertionError(f"Python accepted {identity}")


def failure_rows(ledger: Ledger, repo: Path, revisions: dict[Revision, str],
                 missing_ticket: str, failed_read: str) -> list[dict[str, object]]:
    base, tip = revisions[Revision.BASE], revisions[Revision.TIP]
    ticket_error = python_failure(ledger, tip, missing_ticket, "unknown ticket missing-ticket")
    blob = git(repo, "rev-parse", f"{base}:{MISSING_BLOB_PATH}")
    (repo / ".git" / "objects" / blob[:2] / blob[2:]).unlink()
    python_failure(ledger, tip, failed_read, blob)
    return [
        {"name": "missing direct ticket", "id": missing_ticket, "diagnostics": [ticket_error]},
        {"name": "failed base source read", "id": failed_read,
         "diagnostics": ["rename applied", MISSING_BLOB_PATH, base, blob]},
    ]


def malformed_ticket_requests(root: Path, repo: Path, revisions: dict[Revision, str],
                              cases: Cases) -> list[dict[str, object]]:
    """Use a separate ledger for each malformed ticket field."""
    requests: list[dict[str, object]] = []
    ticket = "malformed-ticket"
    for fixture, source_case, name, claim, corrupt in (
        ("malformed_rules", "only new file", "file rename with malformed ticket rules", "",
         "UPDATE tickets SET rules = ? WHERE id = ?"),
        ("malformed_finding_ids", "field", "qualified rename with malformed ticket finding_ids", ticket,
         "UPDATE tickets SET finding_ids = ? WHERE id = ?"),
    ):
        source = next(row for row in cases.findings.values() if row.name == source_case)
        evidence = Cases()
        identity = evidence.add(replace(source, name=name, build_ticket=claim))
        evidence.tickets.append(Ticket(ticket, (identity,)))
        database = root / f"{fixture}.sqlite"
        with Ledger(repo, database):
            pass
        seed_ledger(database, revisions, evidence)
        with closing(sqlite3.connect(database)) as connection, connection:
            connection.execute(corrupt, ("[", ticket))
        with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
            readings = answers(ledger, revisions[Revision.TIP], evidence, [identity])
        requests.append({"ledger": fixture, "policy": "omitted", "cases": readings})
    return requests


def main(root: Path) -> None:
    repo, database = root / "repo", root / "ledger.sqlite"
    policy = root / "rename_policy.toml"
    policy.write_text('inert_attributes = ["allow"]\nstd_macros = ["println", "vec"]\n', encoding="utf-8")
    renames.__file__ = str(policy.with_name("approved_renames.py"))
    cases = Cases()
    binding_rows(cases)
    ticket_rows(cases)
    companion_rows(cases)
    batch_rows(cases)
    first, *remaining = cases.findings
    policy_ids = [first, UNKNOWN, *reversed(remaining), first]
    file_rows(cases)
    qualified_rows(cases)
    no_policy_ids = sorted(cases.findings.keys() - set(policy_ids), reverse=True)
    assert_edit_proof(cases)
    cases.source("src/missing-ticket.rs", LOCAL, EDITED)
    missing_ticket = cases.add(Finding("missing direct ticket", "src/missing-ticket.rs", LOCAL.splitlines()[1],
                                       "error", build_ticket="missing-ticket"))
    cases.source(MISSING_BLOB_PATH, MISSING_BLOB_SOURCE, LOCAL)
    failed_read = cases.add(Finding("failed base source read", MISSING_BLOB_PATH, LOCAL.splitlines()[1],
                                    "error", anchor=Revision.TIP, build_ticket="bad-source"))
    cases.tickets.append(Ticket("bad-source", (failed_read,)))
    revisions = source_revisions(repo, cases)
    with Ledger(repo, database):
        pass
    seed_ledger(database, revisions, cases)
    tip = revisions[Revision.TIP]
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        readings = answers(ledger, tip, cases, policy_ids)
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        renames.__file__ = str(root / "absent" / "approved_renames.py")
        no_policy = answers(ledger, tip, cases, no_policy_ids)
    malformed = malformed_ticket_requests(root, repo, revisions, cases)
    renames.__file__ = str(policy.with_name("approved_renames.py"))
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        failures = failure_rows(ledger, repo, revisions, missing_ticket, failed_read)
    requests = [{"ledger": "main", "policy": "supplied", "cases": readings},
                {"ledger": "main", "policy": "omitted", "cases": no_policy}, *malformed]
    print(json.dumps({"synced_tip": tip, "requests": requests, "failures": failures}, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
