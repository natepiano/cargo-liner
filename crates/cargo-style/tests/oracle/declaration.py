"""Read declaration outcomes, site evidence and approvals from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import sys
from collections.abc import Callable
from contextlib import closing
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath

from outside_rename import (
    OutsideFixture,
    add_child_cases,
    add_direct_cases,
    add_map_cases,
    add_owner_cases,
    add_search_cases,
    add_type_cases,
)
from same_declaration import (
    FindingCase,
    RelocationFixture,
    SourceVersions,
    add_destination_cases,
    add_guard_cases,
    add_statement_cases,
    add_trait_cases,
    commit,
    create_history,
    declaration_fields,
    git,
    write_files,
    write_ledger,
)
from scripts.style import style_ledger
from scripts.style.finding_sites import FilingEligibleWithoutHistory, NeedsJudgment, UnchangedSite

APPROVED = "decisions/approved.md"
DECLINED = "decisions/declined.md"
UNANSWERED = "decisions/./unanswered.md"
UNOBSERVED = "decisions/missing.md"
NEGATIVE_FOLLOW = "built_trait_negative_follow"
ANSWERS = {APPROVED: "approved", DECLINED: "declined", UNANSWERED: "unanswered", "decision.md": "approved"}


class Unobserved(Exception):
    """The approvals file does not answer a decision that the reading asked about."""

    path: str

    def __init__(self, path: str) -> None:
        super().__init__(path)
        self.path = path


@dataclass(frozen=True)
class RenameCase:
    """A held finding of the rename fixture with the edit and decision it carries."""

    finding: FindingCase
    kind: str
    before: str
    after: str
    proposal: str


class RenameFixture:
    """Give each rename an isolated crate, so a built rename reaches only its own findings."""

    def __init__(self) -> None:
        self.before: dict[str, str] = {}
        self.after: dict[str, str] = {}
        self.crates: list[str] = []
        self.cases: list[RenameCase] = []

    def crate(self, name: str, before: str, after: str, *, prior: dict[str, str] | None = None,
              added: dict[str, str] | None = None) -> str:
        """Write module `a` of a new crate, with more modules before and after the rename."""
        crate = f"crates/{name}"
        self.crates.append(crate)
        modules = sorted({"a", *(prior or {}), *(added or {})})
        self.before[f"{crate}/Cargo.toml"] = f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
        self.before[f"{crate}/src/lib.rs"] = "".join(f"mod {module};\n" for module in modules)
        self.before[f"{crate}/src/a.rs"] = before
        self.after[f"{crate}/src/a.rs"] = after
        for files, entries in ((self.before, prior), (self.after, added)):
            for module, text in (entries or {}).items():
                files[f"{crate}/src/{module}.rs"] = text
        return f"{crate}/src/a.rs"

    def finding(self, label: str, path: str, line: int, *, state: str = "filed", judging: str = "",
                kind: str = "rename", before: str = "", after: str = "", proposal: str = APPROVED) -> None:
        text = self.before[path].splitlines()[line - 1].strip()
        case = FindingCase(label, path, text, "", line=line, state=state, judging=judging)
        self.cases.append(RenameCase(case, kind, before, after, proposal))

    def own_rename(self, label: str, proposal: str) -> None:
        old, new = f"old_{label}", f"new_{label}"
        path = self.crate(label, f"fn {old}() {{}}\n", f"fn {new}() {{}}\n")
        self.finding(label, path, 1, before=old, after=new, proposal=proposal)

    def built_owner(self, label: str, proposal: str, state: str = "unfiled") -> None:
        source = "struct OldType;\nimpl OldType {\n    fn member() {}\n}\n"
        path = self.crate(label, source, source.replace("OldType", "NewType"))
        self.finding(f"{label} built", path, 1, state="built", judging="merged",
                     before="OldType", after="NewType", proposal=proposal)
        self.finding(label, path, 3, state=state, kind="comment", before="member", after="member")

    def built_trait(self, label: str) -> None:
        """A built rename of the implemented trait renames the trait that holds the member."""
        source = ("pub trait OldTrait {\n    fn member();\n}\npub struct Thing;\n"
                  "impl OldTrait for Thing {\n    fn member() {}\n}\n")
        path = self.crate(label, source, source.replace("OldTrait", "NewTrait"))
        self.finding(f"{label} built", path, 1, state="built", judging="merged",
                     before="OldTrait", after="NewTrait", proposal=APPROVED)
        self.finding(label, path, 6, kind="comment", before="member", after="member")

    def built_module_owner(self, label: str) -> None:
        """A module-qualified built rename renames an owner that another module declares."""
        owner = "pub struct OldType;\n"
        user = "use crate::b::OldType;\nimpl OldType {\n    fn member() {}\n}\n"
        path = self.crate(label, user, user.replace("OldType", "NewType"), prior={"b": owner},
                          added={"b": owner.replace("OldType", "NewType")})
        self.finding(f"{label} built", path, 1, state="built", judging="merged",
                     before="b::OldType", after="b::NewType", proposal=APPROVED)
        self.finding(label, path, 3, kind="comment", before="member", after="member")

    def scoped_pair(self, label: str) -> None:
        """A scoped built pair that names the outside rename's target keeps the filed refusal."""
        source = "fn old_pair() {\n    7;\n}\n"
        path = self.crate(label, source, "// The source item moved.\n",
                          added={"b": source.replace("old_pair", "new_pair")})
        self.finding(f"{label} built", path, 1, state="built", judging="merged",
                     before="c::old_pair", after="c::new_pair", proposal=APPROVED)
        self.finding(label, path, 1, kind="comment", before="old_pair", after="old_pair")

    def history(self, repo: Path) -> SourceVersions:
        for key, value in (
            ("user.name", "Declaration Test"),
            ("user.email", "declaration@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
            ("maintenance.auto", "false"),
            ("core.autocrlf", "false"),
        ):
            _ = git(repo, "config", key, value)
        fixed = {
            "Cargo.toml": "[workspace]\nmembers = " + json.dumps(self.crates) + "\n",
            "docs/style/names.md": "---\ntier: C\nfamily: names\ninput_scope: file\n---\n\nUse the fixture rule.\n",
        }
        write_files(repo, fixed | self.before)
        read = commit(repo, "Record the findings")
        write_files(repo, self.after)
        return SourceVersions(read, commit(repo, "Rename the declarations"), read)


def add_rename_cases(fixture: RenameFixture) -> None:
    fixture.own_rename("approved", APPROVED)
    fixture.own_rename("declined", DECLINED)
    fixture.own_rename("unanswered", UNANSWERED)
    fixture.own_rename("unobserved", UNOBSERVED)
    fixture.built_owner("built_approved", APPROVED)
    fixture.built_owner("built_declined", DECLINED)
    fixture.built_owner("built_declined_filed", DECLINED, "filed")
    fixture.built_owner("built_unobserved", UNOBSERVED)
    fixture.built_trait("built_trait")
    fixture.built_trait(NEGATIVE_FOLLOW)
    fixture.built_module_owner("built_module_owner")
    fixture.scoped_pair("scoped_pair")


def write_rename_ledger(repo: Path, root: Path, versions: SourceVersions, cases: dict[str, RenameCase]) -> None:
    write_ledger(repo, root, versions, {identity: case.finding for identity, case in cases.items()})
    with closing(sqlite3.connect(root / "ledger.sqlite")) as db, db:
        # A built rename is judged at the checkpoint that holds the new name.
        _ = db.execute("UPDATE tickets SET checkpoint=? WHERE id='read-merged'", (versions.tip,))
        for identity, case in cases.items():
            ticket = f"read-{case.finding.judging}" if case.finding.judging else ""
            _ = db.execute("UPDATE held_findings SET kind=?,before=?,after=?,proposal=?,ticket=? WHERE finding_id=?",
                       (case.kind, case.before, case.after, case.proposal, ticket, identity))
            if case.finding.label == f"{NEGATIVE_FOLLOW} built":
                _ = db.execute(
                    "UPDATE held_findings SET followed_from=?,followed_from_line=-1,"
                    "followed_from_commit=?,followed_at=? WHERE finding_id=?",
                    (case.finding.source, versions.read, versions.tip, identity),
                )


def outside_evidence(evidence: object) -> str:
    if isinstance(evidence, style_ledger.NoOutsideEvidence):
        return "no_outside_evidence"
    return "outside_rename_not_proven"


def outcome_fields(outcome: style_ledger.DeclarationOutcome) -> dict[str, object]:
    match outcome:
        case style_ledger.DeclarationNotRead():
            return {"status": "declaration_not_read"}
        case style_ledger.DeclarationInPlace():
            return {"status": "declaration_in_place"}
        case style_ledger.FindingSite():
            return {"status": "finding_site"}
        case style_ledger.DeclarationMoved(declaration=declaration, path=path, line=line):
            return {"status": "declaration_moved", "declaration": declaration_fields(declaration),
                    "path": path, "line": line}
        case style_ledger.DeclarationRemoved(declaration=declaration, evidence=evidence):
            return {"status": "declaration_removed", "declaration": declaration_fields(declaration),
                    "evidence": outside_evidence(evidence)}
        case style_ledger.DeclaredInSeveralFiles(declaration=declaration, paths=paths, evidence=evidence):
            return {"status": "declared_in_several_files", "declaration": declaration_fields(declaration),
                    "paths": list(paths), "evidence": outside_evidence(evidence)}
        case style_ledger.DeclaredInAnotherCrate(declaration=declaration, path=path, evidence=evidence):
            return {"status": "declared_in_another_crate", "declaration": declaration_fields(declaration),
                    "path": path, "evidence": outside_evidence(evidence)}
        case style_ledger.DeclarationRenamed(declaration=declaration, renamed=renamed, path=path, line=line):
            return {"status": "declaration_renamed", "declaration": declaration_fields(declaration),
                    "renamed": declaration_fields(renamed), "path": path, "line": line}
        case style_ledger.RenameAwaitingApproval():
            return {"status": "rename_awaiting_approval"}
        case style_ledger.DeclarationGoneFromFile(declaration=declaration, evidence=evidence):
            return {"status": "declaration_gone_from_file", "declaration": declaration_fields(declaration),
                    "evidence": outside_evidence(evidence)}
        case style_ledger.KeptForOpenBuild(ticket=ticket):
            return {"status": "kept_for_open_build", "ticket": ticket}
        case style_ledger.TraitIdentityUncertain(declaration=declaration):
            return {"status": "trait_identity_uncertain", "declaration": declaration_fields(declaration)}
        case style_ledger.OutsideRenameUnresolved(declaration=declaration):
            return {"status": "outside_rename_unresolved", "declaration": declaration_fields(declaration)}
        case style_ledger.StatementNotFollowed(declaration=declaration, path=path, reason=reason):
            return {"status": "statement_not_followed", "declaration": declaration_fields(declaration),
                    "path": path, "reason": str(reason)}
        case _:
            raise AssertionError(f"Unexpected declaration outcome: {outcome}")


def evidence_fields(evidence: object) -> dict[str, object]:
    match evidence:
        case UnchangedSite(path=path, line=line, anchor=anchor):
            return {"status": "unchanged", "site": {"path": path, "line": line}, "anchor": asdict(anchor)}
        case FilingEligibleWithoutHistory(path=path, line=line):
            return {"status": "filing_eligible_without_history", "path": path, "line": line,
                    "reason": evidence.reason}
        case NeedsJudgment(reason=reason):
            return {"status": "needs_judgment", "reason": reason}
        case _:
            raise AssertionError(f"Unexpected site evidence: {evidence}")


def recorder(read: dict[str, str]) -> Callable[[Path], bool]:
    answers = {str(PurePosixPath(path)): answer for path, answer in ANSWERS.items()}

    def approved(path: Path) -> bool:
        key = str(PurePosixPath(path))
        if key not in answers:
            raise Unobserved(key)
        read[key] = answers[key]
        return answers[key] == "approved"

    return approved


def read_answers(repo: Path, root: Path, labels: dict[str, str]) -> list[dict[str, object]]:
    answers: list[dict[str, object]] = []
    with style_ledger.Ledger(repo, root / "ledger.sqlite", access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
        tip = ledger._synced_tip()
        for identity, label in labels.items():
            read: dict[str, str] = {}
            try:
                outcome, = ledger.declaration_reading((identity,), recorder(read)).outcomes
            except Unobserved as stop:
                declaration: dict[str, object] = {"status": "approval_unobserved", "path": stop.path}
                evidence: dict[str, object] = {"status": "approval_unobserved"}
            else:
                declaration = outcome_fields(outcome)
                evidence = evidence_fields(ledger._follow_evidence[(outcome.finding, tip.commit)])
            approvals = [{"path": path, "answer": read[path]} for path in sorted(read)]
            answers.append({"name": label, "id": identity, "answer": {
                "declaration": declaration, "site_evidence": evidence, "approvals": approvals}})
    return answers


def new_repo(root: Path) -> Path:
    repo = root / "repo"
    repo.mkdir(parents=True)
    _ = git(repo, "init", "-q", "--object-format=sha1", "--initial-branch=main")
    return repo


def write_approvals(root: Path) -> None:
    _ = (root / "approvals.json").write_text(json.dumps({"decisions": ANSWERS}), encoding="utf-8")


def relocation(root: Path) -> list[dict[str, object]]:
    repo = new_repo(root)
    fixture = RelocationFixture()
    add_guard_cases(fixture)
    add_destination_cases(fixture)
    add_trait_cases(fixture)
    add_statement_cases(fixture)
    versions = create_history(repo, fixture)
    cases = {f"{index:016x}": case for index, case in enumerate(fixture.cases, 1)}
    write_ledger(repo, root, versions, cases)
    write_approvals(root)
    return read_answers(repo, root, {identity: case.label for identity, case in cases.items()})


def outside(root: Path) -> list[dict[str, object]]:
    repo = new_repo(root)
    fixture = OutsideFixture()
    add_direct_cases(fixture)
    add_search_cases(fixture)
    add_map_cases(fixture)
    add_type_cases(fixture)
    add_child_cases(fixture)
    add_owner_cases(fixture)
    versions = fixture.history(repo)
    cases = {f"{index:016x}": case for index, case in enumerate(fixture.cases, 1)}
    write_ledger(repo, root, versions, {identity: case.finding for identity, case in cases.items()})
    write_approvals(root)
    return read_answers(repo, root, {identity: case.finding.label for identity, case in cases.items()})


def renames(root: Path) -> list[dict[str, object]]:
    repo = new_repo(root)
    fixture = RenameFixture()
    add_rename_cases(fixture)
    versions = fixture.history(repo)
    cases = {f"{index:016x}": case for index, case in enumerate(fixture.cases, 1)}
    write_rename_ledger(repo, root, versions, cases)
    write_approvals(root)
    held = {identity: case.finding.label for identity, case in cases.items() if case.finding.state != "built"}
    return read_answers(repo, root, held)


def main(root: Path) -> None:
    fixtures = {"relocation": relocation, "outside": outside, "renames": renames}
    replies = {name: read(root / name) for name, read in fixtures.items()}
    print(json.dumps(replies, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
