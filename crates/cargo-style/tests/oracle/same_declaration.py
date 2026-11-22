"""Read the same-declaration prefix from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
from collections.abc import Callable
from contextlib import closing
from dataclasses import asdict, dataclass, replace
from pathlib import Path

from scripts.style import declarations, rules, style_ledger
from scripts.style.finding_sites import NeedsJudgment, UnchangedSite

ALPHA = "crates/alpha/src/"
BETA = "crates/beta/src/"
RULE_PATH = "docs/style/names.md"
RULE_ID = f"local:{RULE_PATH}"
STAMP = "2026-01-01T00:00:00+00:00"
UNKNOWN = "f" * 16
MISSING_COMMIT = "f" * 40
REMOVED = "fn unrelated() {}\n"
CODE = "    let item = read(2);"
MISSING_CANDIDATE = "missing_candidate_destination_blob"
MISSING_STATEMENT = declarations.StatementRefusal.TEXT_NOT_FOUND
REPEATED_STATEMENT = declarations.StatementRefusal.TEXT_REPEATED


@dataclass(frozen=True)
class FindingCase:
    """A saved finding and the expected branch of the Python proof."""

    label: str
    source: str
    text: str
    status: str
    line: int = 1
    state: str = "unfiled"
    anchor: str = ""
    judging: str = ""
    build: str = ""
    destination: str = ""
    destination_line: int = 0
    reason: str = ""


@dataclass(frozen=True)
class SourceVersions:
    """The commits containing the source before and after relocation."""

    read: str
    tip: str
    unrelated: str


class ReachedBuildRenames(Exception):
    """The Python proof reached the end of the same-declaration step."""

    def __init__(self, declaration: declarations.Declaration) -> None:
        super().__init__(declaration.name)
        self.declaration = declaration


class RelocationLedger(style_ledger.Ledger):
    """Stop the Python proof at the boundaries of this phase."""

    reached_item_match: bool = False

    def _site_after_refusal(
        self, outcome: style_ledger.DeclarationNotRead | style_ledger.DeclarationInPlace,
        tip: style_ledger.RepositoryTip,
    ) -> style_ledger.DeclarationOutcome:
        self.reached_item_match = isinstance(outcome, style_ledger.DeclarationInPlace)
        return outcome

    def _built_renaming(
        self, finding: style_ledger.HeldFinding, read: str, original: declarations.DeclaredItem,
        tip: style_ledger.RepositoryTip, approved: Callable[[Path], bool],
        renames: tuple[style_ledger.BuiltRename, ...],
    ) -> style_ledger.BuiltRenaming:
        raise ReachedBuildRenames(original.declaration)


def git(repo: Path, *arguments: str, input: str | None = None) -> str:
    return subprocess.run(
        ("git", "-C", str(repo), *arguments),
        input=input, check=True, capture_output=True, text=True,
    ).stdout.strip()


def commit(repo: Path, title: str) -> str:
    git(repo, "add", "--all")
    git(repo, "commit", "-qm", title)
    return git(repo, "rev-parse", "HEAD")


def write_files(repo: Path, entries: dict[str, str]) -> None:
    for path, source in entries.items():
        file = repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(source.encode("utf-8", errors="surrogateescape"))


class RelocationFixture:
    """Describe the fixture files and the findings at their original coordinates."""

    def __init__(self) -> None:
        self.before: dict[str, str] = {}
        self.after: dict[str, str] = {}
        self.cases: list[FindingCase] = []

    def source(self, name: str, before: str, after: str, text: str,
               status: str, *, line: int = 1, build: str = "", destination: str = "",
               destination_line: int = 0, reason: str = "") -> FindingCase:
        path = f"{ALPHA}{name}.rs"
        self.before[path] = before
        self.after[path] = after
        case = FindingCase(name, path, text, status, line=line, build=build, destination=destination,
                           destination_line=destination_line, reason=reason)
        self.cases.append(case)
        return case

    def move(
        self, name: str, source: str, target: str, text: str, status: str = "moved",
        *, line: int = 1, destination_line: int = 0, reason: str = "",
    ) -> FindingCase:
        path = f"{ALPHA}{name}_destination.rs"
        self.after[path] = target
        return self.source(name, source, REMOVED, text, status, line=line, destination=path,
                           destination_line=destination_line, reason=reason)


def add_guard_cases(fixture: RelocationFixture) -> None:
    unchanged = fixture.source("unnamed_unchanged", "use crate::Type;\n", "use crate::Type;\n",
                               "use crate::Type;", "in_place")
    for state in ("waived", "built", "withdrawn"):
        fixture.cases.append(replace(unchanged, label=state, status="not_read", state=state))
    for state in ("assigned", "checkpointed"):
        fixture.cases.append(replace(unchanged, label=f"judging {state}", status="not_read", judging=state))
    fixture.cases.extend((
        replace(unchanged, label="filed with an assigned judging ticket", state="filed", judging="assigned"),
        replace(unchanged, label="missing anchor", status="not_read", anchor=MISSING_COMMIT),
        replace(unchanged, label="non-ancestor anchor", status="not_read", anchor="unrelated"),
        FindingCase("Markdown", "docs/note.md", "Review this text.", "not_read"),
    ))
    fixture.source("unnamed_edited", "use crate::Type;\n", "use crate::Other;\n", "use crate::Type;", "not_read")
    header = "fn stays() {}"
    fixture.source("retained_item", f"{header}\n", f"// Move the header.\n{header}\n", header, "in_place")


def add_destination_cases(fixture: RelocationFixture) -> None:
    header = "fn header() {}"
    fixture.move("header", f"{header}\n", f"// New heading.\n{header}\n", header, destination_line=2)
    text = "fn replacement_source() {}"
    fixture.move("replacement_source", f"// Invalid byte: \udcff\n{text}\n",
                 f"{text}\n", text, line=2, destination_line=1)
    owner = "struct Fields {\n    item: i32,\n}\n"
    fixture.move("field", owner, "// New heading.\n" + owner, "item: i32,", line=2, destination_line=3)

    twin_path = f"{ALPHA}twin.rs"
    twin = "fn gains() {}\n"
    fixture.before[twin_path] = twin
    fixture.after[twin_path] = twin + "fn helper() {}\n"
    fixture.move("gained_only", twin, twin, twin.strip(), destination_line=1)

    cross_path = f"{BETA}cross.rs"
    cross = "fn cross() {}\n"
    fixture.after[cross_path] = cross
    unfiled = fixture.source("cross", cross, REMOVED, cross.strip(), "moved",
                              destination=cross_path, destination_line=1)
    fixture.cases.append(replace(unfiled, label="filed cross-crate", state="filed", build="assigned",
                                 status="another_crate", destination_line=0))

    ignored = "fn ignored() {}\n"
    fixture.after[f"{ALPHA}ignored_destination.rs"] = ignored
    fixture.source("ignored_source", ignored, REMOVED, ignored.strip(), "no_destination")

    for label, destination in (("reversed_range_tail", "src/candidate0.rs"),
                               ("negated_empty_range", "d.rs")):
        text = f"fn {label}() {{}}"
        fixture.after[destination] = text + "\n"
        fixture.source(label, text + "\n", REMOVED, text, "no_destination")

    older_twin = "fn earlier() {}\n"
    fixture.before[f"{ALPHA}earlier_twin.rs"] = older_twin
    fixture.after[f"{ALPHA}earlier_twin.rs"] = older_twin + "fn changed() {}\n"
    fixture.source("earlier_source", older_twin, REMOVED, older_twin.strip(), "no_destination")

    moving = "fn work_for_build() {}\n"
    open_build = fixture.move("build_move", moving, moving, moving.strip(),
                              status="open_build", destination_line=1)
    fixture.cases[-1] = replace(open_build, build="assigned")
    for state in ("merged", "dropped"):
        fixture.cases.append(replace(open_build, label=f"{state} build", status="moved", build=state))

    gone = "fn gone() {}\n"
    open_without_move = fixture.source("no_gain_open_build", gone, REMOVED, gone.strip(),
                                       "open_build", build="checkpointed")
    fixture.cases.append(replace(open_without_move, label="no file names the declaration",
                                 status="no_destination", build=""))

    split = "fn split() {}\n"
    fixture.after[f"{ALPHA}split_first.rs"] = split
    fixture.after[f"{ALPHA}split_second.rs"] = split
    fixture.source("split", split, REMOVED, split.strip(), "several_destinations", build="missing")
    twice = "fn twice() {}\n"
    fixture.move("twice", twice, twice * 2, twice.strip(), "several_destinations")


def add_trait_cases(fixture: RelocationFixture) -> None:
    def member(name: str, trait: str) -> str:
        return f"impl {trait} for Type {{\n    fn {name}() {{}}\n}}\n"

    relative = "relative_trait"
    fixture.move(relative, member(relative, "crate::first::Contract<u8>"),
                 member(relative, "super::first::Contract<u8>"), f"fn {relative}() {{}}",
                 line=2, destination_line=2)
    cases = (
        ("alias", "crate::first::Contract", "Alias", "use crate::first::Contract as Alias;\n", "in_place"),
        ("unreadable_trait", "<Type as Family>::Contract", "Contract", "", "trait_identity_uncertain"),
        ("glob", "crate::first::Contract", "Contract", "use external::*;\n", "trait_identity_uncertain"),
        ("blocked_alias", "crate::first::Contract", "Alias",
         "#[cfg(test)] use crate::first::Contract as Alias;\n", "trait_identity_uncertain"),
        ("attributed_import", "crate::first::Contract", "crate::first::Contract",
         "#[cfg_attr(any(), helper(use separate;))]\nuse other::Contract as Alias;\n", "trait_identity_uncertain"),
        ("duplicate_homes", "Contract", "crate::first::Contract", "", "trait_identity_uncertain"),
        ("qualified_paths", "crate::first::Contract", "crate::second::Contract", "", "no_destination"),
        ("different_arguments", "crate::first::Contract<u8>", "crate::first::Contract<u16>", "", "no_destination"),
    )
    for name, old_trait, new_trait, imports, status in cases:
        before = member(name, old_trait)
        after = imports + member(name, new_trait)
        if name == "alias":
            fixture.source(name, before, after, f"fn {name}() {{}}", status, line=2)
        else:
            fixture.move(name, before, after, f"fn {name}() {{}}", status, line=2)


def add_statement_cases(fixture: RelocationFixture) -> None:
    cases = (
        ("statement", f"    let other = read(1);\n{CODE}\n", "moved", 3, ""),
        ("missing_statement", "    let item = read(3);\n", "statement_not_followed", 0, MISSING_STATEMENT),
        ("repeated_statement", f"{CODE}\n{CODE}\n", "statement_not_followed", 0, REPEATED_STATEMENT),
        ("nested_statement", f"    fn nested() {{\n    {CODE}\n    }}\n", "statement_not_followed", 0, MISSING_STATEMENT),
        ("comment_or_string", f'    // {CODE}\n    let note = "{CODE.strip()}";\n',
         "statement_not_followed", 0, MISSING_STATEMENT),
        ("crlf_comment", f"    // {CODE}\n{CODE} // New note.\n", "moved", 3, ""),
    )
    for name, body, status, line, reason in cases:
        text = CODE + " // Old note." if name == "crlf_comment" else CODE
        source = f"fn {name}() {{\n{text}\n}}\n"
        destination = f"fn {name}() {{\n{body}}}\n"
        if name == "crlf_comment":
            source = source.replace("\n", "\r\n")
            destination = destination.replace("\n", "\r\n")
        fixture.move(name, source, destination, text, status, line=2, destination_line=line, reason=reason)


def create_history(repo: Path, fixture: RelocationFixture) -> SourceVersions:
    for key, value in (
        ("user.name", "Relocation Test"),
        ("user.email", "relocation@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("maintenance.auto", "false"),
        ("core.autocrlf", "false"),
    ):
        git(repo, "config", key, value)
    fixed = {
        "Cargo.toml": '[workspace]\nmembers = ["crates/alpha", "crates/beta"]\n',
        "crates/alpha/Cargo.toml": '[package]\nname = "alpha"\nversion = "0.1.0"\n',
        "crates/beta/Cargo.toml": '[package]\nname = "beta"\nversion = "0.1.0"\n',
        f"{ALPHA}lib.rs": (
            "mod first;\nmod second;\nmod duplicate_homes;\nmod duplicate_homes_destination;\n"
            + "mod relative_trait_destination;\n"
        ),
        f"{ALPHA}first.rs": "pub trait Contract<T> {}\n",
        f"{ALPHA}second.rs": "pub trait Contract<T> {}\n",
        f"{BETA}lib.rs": "pub fn beta() {}\n",
        ".claude/config/berth.toml": (
            f'ignore = ["{ALPHA}ignored_destination.rs", "src/candidate[z-a0].rs", "[!z-a].rs"]\n'
        ),
        RULE_PATH: "---\ntier: C\nfamily: names\ninput_scope: file\n---\n\nUse the fixture rule.\n",
        "docs/note.md": "Review this text.\n",
    }
    write_files(repo, fixed | fixture.before)
    read = commit(repo, "Read the original declarations")
    unrelated = git(repo, "commit-tree", f"{read}^{{tree}}", "-m", "An unrelated root")
    write_files(repo, fixture.after)
    return SourceVersions(read, commit(repo, "Sync the new declarations"), unrelated)


def write_ledger(repo: Path, root: Path, versions: SourceVersions, cases: dict[str, FindingCase]) -> None:
    database = root / "ledger.sqlite"
    with style_ledger.Ledger(repo, database):
        pass
    pin = git(repo, "rev-parse", f"{versions.read}:{RULE_PATH}")
    rule_text = (repo / RULE_PATH).read_bytes()
    with closing(sqlite3.connect(database)) as db, db:
        db.execute("INSERT INTO sync (id,tip,synced_at) VALUES (1,?,?)", (versions.tip, STAMP))
        db.execute("INSERT INTO rule_texts (blob,text) VALUES (?,?)", (pin, rule_text))
        for state in ("assigned", "checkpointed", "merged", "dropped"):
            for kind in ("read", "build"):
                ticket = f"{kind}-{state}"
                db.execute(
                    "INSERT INTO tickets (id,unit,kind,family,rule,crate,files,sites,rules,rule_blobs,"
                    "state,branch,checkpoint,base,created_at,updated_at) VALUES (?,?,?,'names',?,"
                    "'alpha','[]','[]','[]','{}',?,'main',?,?,?,?)",
                    (ticket, ticket, kind, RULE_ID, state, versions.read, versions.read, STAMP, STAMP),
                )
        for identity, case in cases.items():
            anchor = versions.unrelated if case.anchor == "unrelated" else case.anchor or versions.read
            db.execute(
                "INSERT INTO held_findings (finding_id,rule,path,line,text,kind,before,after,rule_blob,ticket,state,"
                "proposal,created_at,updated_at,anchor_commit,build_ticket) "
                "VALUES (?,?,?,?,?,'rename','item','better_item',?,?,?,'decision.md',?,?,?,?)",
                (identity, RULE_ID, case.source, case.line, case.text, pin, f"read-{case.judging}", case.state,
                 STAMP, STAMP, anchor, f"build-{case.build}" if case.build else ""),
            )
    definition, = rules.load_rules(repo)
    catalog = {"tier": definition.tier, "family": definition.family, "surface": None, "blob": pin,
               "identities": {pin: definition.judgment_identity}}
    (root / "rules.json").write_text(json.dumps({"rules": {RULE_ID: catalog}}), encoding="utf-8")


def reject_approval(path: Path) -> bool:
    raise AssertionError(f"The same-declaration proof read an approval at {path}")


def declaration_fields(declaration: declarations.Declaration) -> dict[str, object]:
    owner = declaration.owner
    ownership = ({"kind": "impl_or_trait_item", "name": owner.name}
                 if isinstance(owner, declarations.ImplOrTraitItem) else {"kind": "module_item"})
    return {"keyword": declaration.keyword, "name": declaration.name, "owner": ownership}


def same_declaration(
    ledger: RelocationLedger, finding: style_ledger.HeldFinding, tip: style_ledger.RepositoryTip,
) -> dict[str, object]:
    ledger.reached_item_match = False
    try:
        result = ledger._declaration_outcome(finding, tip, frozenset(tip.rust_files), reject_approval, (), ())
    except ReachedBuildRenames as stop:
        return {"status": "no_destination", "declaration": declaration_fields(stop.declaration)}
    match result:
        case style_ledger.DeclarationNotRead():
            return {"status": "not_read"}
        case style_ledger.DeclarationInPlace():
            return {"status": "in_place", "basis": "matching_item" if ledger.reached_item_match else "unchanged_path"}
        case style_ledger.DeclarationMoved(declaration=declaration, path=path, line=line):
            return {"status": "moved", "declaration": declaration_fields(declaration), "path": path, "line": line}
        case style_ledger.DeclaredInSeveralFiles(declaration=declaration, paths=paths):
            return {"status": "several_destinations", "declaration": declaration_fields(declaration),
                    "paths": paths, "reason": "declared in " + ", ".join(paths)}
        case style_ledger.DeclaredInAnotherCrate(declaration=declaration, path=path):
            return {"status": "another_crate", "declaration": declaration_fields(declaration), "path": path,
                    "reason": f"declared in another crate: {path}"}
        case style_ledger.TraitIdentityUncertain(declaration=declaration):
            return {"status": "trait_identity_uncertain", "declaration": declaration_fields(declaration),
                    "reason": "trait identity uncertain"}
        case style_ledger.KeptForOpenBuild(ticket=ticket):
            return {"status": "open_build", "ticket": ticket, "reason": f"open ticket {ticket} builds it"}
        case style_ledger.StatementNotFollowed(declaration=declaration, path=path, reason=reason):
            return {"status": "statement_not_followed", "declaration": declaration_fields(declaration),
                    "path": path, "reason": reason}
        case _:
            raise AssertionError(f"Unexpected outcome outside the same-declaration proof: {result}")


def failure(repo: Path, root: Path, identity: str) -> str:
    with RelocationLedger(repo, root / "ledger.sqlite", access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
        try:
            same_declaration(ledger, ledger._held(identity), ledger._synced_tip())
        except style_ledger.LedgerError as error:
            return str(error)
    raise AssertionError(f"Python returned a reply for failure fixture {identity}")


def read_answers(repo: Path, root: Path, cases: dict[str, FindingCase]) -> list[dict[str, object]]:
    with RelocationLedger(repo, root / "ledger.sqlite", access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
        findings = {item.finding_id: item for item in ledger.held_findings(style_ledger.AllHeldFindings())}
        tip = ledger._synced_tip()
        answers: list[dict[str, object]] = []
        keys = list(reversed(cases))
        for identity in (keys[0], UNKNOWN, *keys[1:], keys[0]):
            if identity == UNKNOWN:
                answers.append({"name": "unknown finding", "answer": {"id": identity, "status": "unknown_finding"}})
                continue
            case = cases[identity]
            finding = findings[identity]
            result = same_declaration(ledger, finding, tip)
            assert result["status"] == case.status, (case.label, result)
            if case.status in {"moved", "another_crate", "statement_not_followed"}:
                assert result["path"] == case.destination, (case.label, result)
            if case.destination_line and case.status == "moved":
                assert result["line"] == case.destination_line, (case.label, result)
            if case.reason:
                assert result["reason"] == case.reason, (case.label, result)
            anchor = ledger.finding_anchor(finding)
            fields = ({"status": "needs_judgment", "reason": anchor.reason} if isinstance(anchor, NeedsJudgment)
                      else {"status": "anchored", **asdict(anchor)})
            site = ledger._site_proof(finding, tip.commit)
            site_fields = ({"status": "unchanged", "site": {"path": site.path, "line": site.line},
                            "anchor": asdict(site.anchor)} if isinstance(site, UnchangedSite)
                           else {"status": "needs_judgment", "reason": site.reason})
            answers.append({"name": case.label, "answer": {"id": identity, **fields, "original_site": site_fields,
                                                           "same_declaration": result}})
        return answers


def check_missing_candidate(
    repo: Path, root: Path, versions: SourceVersions, identity: str, destination: str,
) -> str:
    """Remove only the destination's old blob and prove which read fails."""
    blob = git(repo, "rev-parse", f"{versions.read}:{destination}")
    object_file = repo / ".git/objects" / blob[:2] / blob[2:]
    saved_blob = object_file.read_bytes()
    object_file.unlink()
    try:
        with RelocationLedger(repo, root / "ledger.sqlite", access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
            finding = ledger._held(identity)
            site = ledger._site_proof(finding, versions.tip)
            assert site == NeedsJudgment("unchanged source site not proved"), site
        missing_blob = failure(repo, root, identity)
        assert "cat-file" in missing_blob and blob in missing_blob, missing_blob
    finally:
        object_file.write_bytes(saved_blob)
    return blob


def main(root: Path) -> None:
    repo = root / "repo"
    fixture = RelocationFixture()
    add_guard_cases(fixture)
    add_destination_cases(fixture)
    add_trait_cases(fixture)
    declaration = "fn moves_to_existing_file() {}"
    target = fixture.move(MISSING_CANDIDATE, declaration + "\n", declaration + "\n",
                          declaration, destination_line=1)
    fixture.before[target.destination] = "fn original_destination_item() {}\n"
    add_statement_cases(fixture)

    versions = create_history(repo, fixture)
    cases = {f"{index:016x}": case for index, case in enumerate(fixture.cases, 1)}
    missing_build_id = "b" * 16
    missing_candidate_id = next(identity for identity, case in cases.items() if case is target)
    missing_build = FindingCase("missing build", f"{ALPHA}no_gain_open_build.rs",
                                "fn gone() {}", "error", build="missing")
    write_ledger(repo, root, versions, cases | {missing_build_id: missing_build})
    answers = read_answers(repo, root, cases)
    missing_ticket = failure(repo, root, missing_build_id)
    assert missing_ticket == "unknown ticket build-missing", missing_ticket
    blob = check_missing_candidate(repo, root, versions, missing_candidate_id, target.destination)
    print(json.dumps({
        "synced_tip": versions.tip,
        "cases": answers,
        "missing_build": {"id": missing_build_id, "diagnostic": missing_ticket},
        "missing_candidate": {"id": missing_candidate_id, "blob": blob},
    }, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
