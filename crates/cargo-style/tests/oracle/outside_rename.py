"""Read outside rename proofs from the pinned Python ledger."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from dataclasses import asdict, dataclass
from pathlib import Path

from same_declaration import (
    UNKNOWN,
    FindingCase,
    RelocationLedger,
    SourceVersions,
    commit,
    declaration_fields,
    git,
    same_declaration,
    write_files,
    write_ledger,
)
from scripts.style import declarations, style_ledger
from scripts.style.finding_sites import NeedsJudgment, UnchangedSite

Proof = style_ledger.OutsideRenameProven | style_ledger.OutsideRenameNotProven
REMOVED = "// The source item moved or was removed.\n"
ORIGINAL = "fn old_name() {\n    7;\n}\n"
RENAMED = ORIGINAL.replace("old_name", "new_name")
ZERO = "zero passing files"
BODY = "body check failed"
EMPTY = "body unreadable or empty"
OWNER = "owner differs"
TRAIT = "trait identity uncertain"
PRIOR = "declaration not gained"
CHILD = "child binding or scope not proved"
UNMAPPED = "mapped site not proved"
PRESENT = "old name still declared in crate"


@dataclass(frozen=True)
class ExpectedProof:
    """The proof variant and evidence that distinguish each fixture."""

    outcome: str
    reasons: tuple[str, ...] = ()
    mapping: str = ""
    line: int = 0


@dataclass(frozen=True)
class OutsideCase:
    """A held finding with its expected relocation and outside branches."""

    finding: FindingCase
    proof: ExpectedProof


def refused(*reasons: str, mapping: str = "") -> ExpectedProof:
    return ExpectedProof("not_proven", reasons, mapping)


def mapped(line: int = 0) -> ExpectedProof:
    return ExpectedProof("mapped_site_proof", line=line)


def gained(line: int = 0) -> ExpectedProof:
    return ExpectedProof("gained_file_proof", line=line)


class OutsideLedger(RelocationLedger):
    """Capture the outside result at each stop in the relocation prefix."""

    outside: Proof | style_ledger.NoOutsideEvidence = style_ledger.NoOutsideEvidence()

    def _built_renaming(
        self, finding: style_ledger.HeldFinding, read: str, original: declarations.DeclaredItem,
        tip: style_ledger.RepositoryTip, approved: Callable[[Path], bool],
        renames: tuple[style_ledger.BuiltRename, ...],
    ) -> style_ledger.BuiltRenaming:
        self.outside = self._outside_rename(
            finding, read, self._declared_file(read, finding.path), original, tip,
            self._changed_since(read, tip.commit),
        )
        return super()._built_renaming(finding, read, original, tip, approved, renames)

    def _outside_refusal_evidence(
        self, finding: style_ledger.HeldFinding, read: str, old_file: declarations.DeclaredFile,
        old: declarations.DeclaredItem, tip: style_ledger.RepositoryTip, changed: frozenset[str],
    ) -> style_ledger.OutsideRenameNotProven:
        evidence = super()._outside_refusal_evidence(finding, read, old_file, old, tip, changed)
        self.outside = evidence
        return evidence


class OutsideFixture:
    """Give each search an isolated crate in one repository."""

    def __init__(self) -> None:
        self.before: dict[str, str] = {}
        self.after: dict[str, str] = {}
        self.cases: list[OutsideCase] = []
        self.crates: list[str] = []
        self.ignored: list[str] = []
        self.deletions: list[str] = []
        self.other: dict[str, str] = {}

    def case(
        self, name: str, before: str, after: str, proof: ExpectedProof, *,
        line: int = 1, prior_files: dict[str, str] | None = None,
        changed_files: dict[str, str] | None = None, other: str = "", other_prior: str = "",
        ignore: str = "", delete: bool = False, source_module: str = "a",
        relocation: str = "no_destination", state: str = "unfiled", build: str = "",
    ) -> None:
        crate = f"crates/{name}"
        self.crates.append(crate)
        source = f"{crate}/src/{source_module}.rs"

        def spellings(text: str) -> str:
            return (text.replace("old_name", f"old_{name}").replace("new_name", f"new_{name}")
                    .replace("fn kept", f"fn kept_{name}"))

        old_files = {source_module: before, **(prior_files or {})}
        new_files = {source_module: after, **(changed_files or {})}
        modules = sorted(name for name in old_files.keys() | new_files.keys() if name != "lib" and name.isidentifier())
        self.before[f"{crate}/Cargo.toml"] = (
            f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n'
        )
        self.before[f"{crate}/src/lib.rs"] = "".join(f"mod {module};\n" for module in modules)
        for files, entries in ((self.before, old_files), (self.after, new_files)):
            for module, content in entries.items():
                files[f"{crate}/src/{module}.rs"] = spellings(content)
        if other:
            self.other[f"crates/other/src/{name}.rs"] = spellings(other)
        if other_prior:
            self.before[f"crates/other/src/{name}.rs"] = spellings(other_prior)
        if ignore:
            self.ignored.append(f"{crate}/src/{ignore}.rs")
        if delete:
            self.deletions.append(source)
        text = spellings(before).splitlines()[line - 1].strip()
        self.cases.append(OutsideCase(FindingCase(name, source, text, relocation, line=line,
                                                  state=state, build=build), proof))

    def history(self, repo: Path) -> SourceVersions:
        for key, value in (
            ("user.name", "Outside Rename Test"),
            ("user.email", "outside-rename@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
            ("maintenance.auto", "false"),
            ("core.autocrlf", "false"),
        ):
            git(repo, "config", key, value)
        fixed = {
            "Cargo.toml": '[workspace]\nmembers = ' + json.dumps([*self.crates, "crates/other"]) + "\n",
            "crates/other/Cargo.toml": '[package]\nname = "other"\nversion = "0.1.0"\n',
            "crates/other/src/lib.rs": "//! Outside the search crate.\n",
            ".claude/config/berth.toml": "ignore = " + json.dumps(self.ignored) + "\n",
            "docs/style/names.md": "---\ntier: C\nfamily: names\ninput_scope: file\n---\n\nUse the fixture rule.\n",
        }
        write_files(repo, fixed | self.before)
        read = commit(repo, "Record the findings")
        write_files(repo, self.after | self.other)
        for path in self.deletions:
            (repo / path).unlink()
        return SourceVersions(read, commit(repo, "Sync the renamed sources"), read)


def add_direct_cases(fixture: OutsideFixture) -> None:
    recursive = "fn old_name() {\n    old_name();\n}\n"
    fixture.case("recursive", recursive, recursive.replace("old_name", "new_name"), mapped(1))
    fixture.case("gained", ORIGINAL, REMOVED, gained(1), changed_files={"b": RENAMED})
    before = "fn old_name() {\n    1;\n\n    2;\n    3;\n    4;\n}\n"
    half = "fn new_name() {\n    1;\n    2;\n    8;\n    9;\n}\n"
    fixture.case("exact_half", before, half, mapped(1))
    fixture.case("below_half", before, half.replace("2;", "6;"), refused(ZERO, BODY))
    fixture.case("empty_body", ORIGINAL, "fn new_name() {}\n", refused(ZERO, EMPTY))
    fixture.case("unreadable_body", ORIGINAL, "fn new_name();\n", refused(ZERO, EMPTY))


def add_search_cases(fixture: OutsideFixture) -> None:
    fixture.case("old_in_crate", ORIGINAL, RENAMED, refused(PRESENT),
                 prior_files={"b": ORIGINAL})
    fixture.case("old_elsewhere", ORIGINAL, RENAMED, mapped(1), other_prior=ORIGINAL)
    fixture.case("ignored_path", ORIGINAL, REMOVED, refused(ZERO), changed_files={"b": RENAMED}, ignore="b")
    fixture.case("other_crate", ORIGINAL, REMOVED, refused(ZERO), other=RENAMED)
    fixture.case("unchanged_path", ORIGINAL, REMOVED, refused(ZERO), prior_files={"b": RENAMED})
    fixture.case("no_candidate", ORIGINAL, REMOVED, refused(ZERO, mapping="mapped_no_declaration"))
    fixture.case("prior_target", ORIGINAL, REMOVED, refused(ZERO, PRIOR),
                 prior_files={"b": RENAMED}, changed_files={"b": "// Changed.\n" + RENAMED})
    fixture.case("several_paths", ORIGINAL, REMOVED, refused("several passing files"),
                 changed_files={"z": RENAMED, "b": RENAMED})
    fixture.case("several_items", ORIGINAL, REMOVED, refused("ambiguous destination items"),
                 changed_files={"b": RENAMED.replace("new_name", "zebra") + RENAMED})
    fixture.case("same_name_conflict", ORIGINAL, RENAMED, refused(PRESENT), relocation="several_destinations",
                 changed_files={"b": ORIGINAL, "c": ORIGINAL})
    fixture.case("cross_crate_conflict", ORIGINAL, RENAMED, refused(UNMAPPED), relocation="another_crate",
                 other=ORIGINAL, state="filed")
    fixture.case("in_place", ORIGINAL, "// Header moved.\n" + ORIGINAL,
                 ExpectedProof("not_applicable"), relocation="in_place")
    fixture.case("open_build", ORIGINAL, RENAMED, ExpectedProof("not_applicable"),
                 relocation="open_build", build="assigned")


def add_map_cases(fixture: OutsideFixture) -> None:
    before = "#[inline]\n#[allow(unused)]\n" + ORIGINAL
    fixture.case("shorter_hunk", before, RENAMED, mapped(1), line=3)
    fixture.case("deleted_source", ORIGINAL, REMOVED, refused(ZERO, mapping="source site deleted"), delete=True)
    fixture.case("no_hunks", ORIGINAL, "// Binary patch\x00\n" + RENAMED,
                 refused(UNMAPPED, mapping="source mapping unreadable"))
    # Git also matches a0.rs when it reads the patch for a[0].rs.
    neighbour = "//! The adjacent source file.\n"
    fixture.case("out_of_range", "#[inline]\n" + ORIGINAL, "#[inline]\n" + RENAMED,
                 refused(UNMAPPED, mapping="mapped site outside destination"), source_module="a[0]",
                 prior_files={"a0": neighbour}, changed_files={"a0": "// Extra line.\n" * 5 + neighbour})
    fixture.case("shared_boundary", ORIGINAL, "fn peer() {} " + RENAMED,
                 refused(UNMAPPED, mapping="several declarations own mapped site"))
    fixture.case("map_to_other_item", ORIGINAL, "const PEER: u8 = 4;\n" + RENAMED,
                 refused(UNMAPPED, mapping="mapped_declaration"))
    # The normalized diff maps line 2 to line 3. Myers deletes line 2.
    before = "#[inline]\n// B\n// B\nfn old_name() {\n    1;\n    2;\n    2;\n    2;\n}\n"
    after = "// B\n#[inline]\n#[inline]\nfn new_name() {\n    1;\n    1;\n    1;\n    2;\n}\n"
    fixture.case("repeated_lines", before, REMOVED, gained(3), line=2, changed_files={"b": after})


def member(owner: str, name: str = "old_name", *, trait: str = "", body: str = "7;") -> str:
    header = f"{trait} for {owner}" if trait else owner
    return f"impl {header} {{\n    fn {name}() {{\n        {body}\n    }}\n}}\n"


def add_type_cases(fixture: OutsideFixture) -> None:
    fixture.case("owner_path", member("crate::left::Holder"), member("crate::right::Holder", "new_name"),
                 refused(ZERO, OWNER), line=2)
    fixture.case("owner_arguments", member("crate::a::Holder<u8>"), member("crate::a::Holder<u16>", "new_name"),
                 refused(ZERO, OWNER), line=2)
    local = "struct Holder<T>(T);\nmod local { pub struct Arg; }\n"
    using = "use crate::a::local::Arg as Alias;\n"
    fixture.case("resolved_argument", local + using + member("crate::a::Holder<Alias>"),
                 local + member("crate::a::Holder<crate::a::local::Arg>", "new_name"), mapped(4), line=5)
    plain = {"lib": "mod a;\n"}
    aliased = {"lib": "extern crate std as alias;\nmod a;\n"}
    for name, prior, changed in (("old_root_extern", aliased, plain), ("new_root_extern", plain, aliased)):
        fixture.case(name, member("crate::a::Holder<alias::String>"),
                     member("crate::a::Holder<alias::String>", "new_name"), refused(ZERO, OWNER), line=2,
                     prior_files=prior, changed_files=changed)
    local_header = "#[cfg(feature = \"one\")]\nstruct Arg;\n"
    fixture.case("conditional_argument", local_header + member("crate::a::Holder<Arg>"),
                 local_header + member("crate::a::Holder<Arg>", "new_name"), refused(ZERO, OWNER), line=4)
    raw_header = "#[r#cfg(all())]\nstruct Arg;\n"
    fixture.case("raw_cfg_argument", raw_header + member("crate::Holder<Arg>"),
                 raw_header + member("crate::Holder<Arg>", "new_name"), mapped(4), line=4)
    fixture.case("ambiguous_argument", "struct Arg;\nstruct Arg;\n" + member("crate::a::Holder<Arg>"),
                 "struct Arg;\nstruct Arg;\n" + member("crate::a::Holder<Arg>", "new_name"),
                 refused(ZERO, OWNER), line=4)

    contract = "trait Contract {\n    fn contract();\n}\n"
    original = contract + member("crate::a::Holder", trait="crate::a::Contract")
    uncertain = contract + "#[cfg(test)]\nuse crate::a::Contract as Alias;\n" + member(
        "crate::a::Holder", "new_name", trait="Alias")
    fixture.case("unresolved_trait", original, uncertain, refused(ZERO, TRAIT), line=5)
    before = member("crate::a::Holder")
    candidate = member("crate::b::Holder", "new_name", body="8;")
    fixture.case("combined_checks", before, REMOVED, refused(ZERO, OWNER, PRIOR, BODY, EMPTY), line=2,
                 prior_files={"b": candidate}, changed_files={
                     "z": member("crate::a::Holder", "empty", body=""),
                     "b": "// Check this earlier candidate.\n" + candidate,
                     "c": member("crate::c::Holder", "changed", body="9;"),
                 })


def add_owner_cases(fixture: OutsideFixture) -> None:
    before = "struct OldOwner {\n    value: u8,\n}\n" + member("crate::a::OldOwner", "kept")
    after = before.replace("OldOwner", "NewOwner")
    fixture.case("kept_child", before, after, mapped(5), line=5)
    fixture.case("renamed_child", before, after.replace("fn kept", "fn fresh"), mapped(5), line=5)
    fixture.case("missing_child", before, after.split("impl", 1)[0], refused(ZERO), line=5)
    fixture.case("duplicate_child", before, after + member("crate::a::NewOwner", "kept"), refused(CHILD), line=5)
    trait = "trait OldContract {\n    fn keep(&self);\n}\n"
    impl_ = member("crate::a::Holder", "kept", trait="crate::a::OldContract")
    before = trait + impl_
    after = before.replace("OldContract", "NewContract")
    fixture.case("renamed_trait", before, after, mapped(5), line=5)
    fixture.case("missing_trait_home", impl_, impl_.replace("OldContract", "NewContract"), refused(CHILD), line=2)
    fixture.case("duplicate_trait_home", trait + before, after, refused(CHILD), line=8)
    fixture.case("missing_trait_child", before, after.split("impl", 1)[0], refused(CHILD), line=5)
    fixture.case("duplicate_trait_child", before, after + impl_.replace("OldContract", "NewContract"),
                 refused(CHILD), line=5)

    label = "nested_trait_crate"
    nested = f"crates/{label}/src/owned"
    module = {"lib": "mod a;\nmod b;\nmod owned;\n", "owned/mod": trait}
    renamed_trait = trait.replace("OldContract", "NewContract")
    fixture.case(label, member("crate::a::Holder", "kept", trait="crate::owned::OldContract"),
                 member("crate::a::Holder", "kept", trait="crate::b::NewContract"), refused(CHILD), line=2,
                 prior_files=module, changed_files={"b": renamed_trait, "owned/mod": renamed_trait})
    fixture.after[f"{nested}/Cargo.toml"] = (
        '[package]\nname = "nested_trait_owner"\nversion = "0.1.0"\n[lib]\npath = "mod.rs"\n'
    )
    fixture.after["Cargo.toml"] = (
        '[workspace]\nmembers = ' + json.dumps([*fixture.crates, "crates/other", nested]) + "\n"
    )


def add_child_cases(fixture: OutsideFixture) -> None:
    statement = "    consume(7);\n"
    before = f"fn old_name() {{\n{statement}    finish();\n}}\n"
    after = before.replace("old_name", "new_name")
    fixture.case("unique_statement", before, after.replace(statement, "    prepare();\n" + statement), mapped(3), line=2)
    fixture.case("repeated_statement", before, after.replace(statement, statement * 2), refused(CHILD), line=2)
    field = "struct old_name {\n    value: u8,\n    other: u8,\n}\n"
    fixture.case("retained_field", field, field.replace("old_name", "new_name"), mapped(2), line=2)
    separated = field.replace("value:", "value\x1f:")
    fixture.case("field_python_whitespace", separated, separated.replace("old_name", "new_name"), mapped(2), line=2)
    fixture.case("field_position", field, field.replace("old_name", "new_name").replace(
        "    value: u8,\n", "    added: i8,\n    value: u8,\n"), refused(CHILD), line=2)
    parameter = "fn old_name(\n    value: u8,\n) {\n    consume(value);\n}\n"
    fixture.case("retained_parameter", parameter, parameter.replace("old_name", "new_name"), mapped(2), line=2)
    block = "fn old_name() {\n    if ready() {\n        let item = 7;\n        consume(item);\n    }\n}\n"
    fixture.case("stable_block", block, block.replace("old_name", "new_name"), mapped(3), line=3)
    fixture.case("changed_block", block, block.replace("old_name", "new_name").replace("ready()", "enabled()"),
                 refused(CHILD), line=3)
    closure = "fn old_name() {\n    consume(\n        |item| item + 1,\n    );\n    finish();\n}\n"
    fixture.case("stable_closure", closure, closure.replace("old_name", "new_name"), mapped(3), line=3)
    fixture.case("changed_closure", closure, closure.replace("old_name", "new_name").replace("consume(", "register("),
                 refused(CHILD), line=3)


def mapped_fields(site: declarations.MappedSite) -> dict[str, object]:
    match site:
        case declarations.MappedDeclaration(path, line, declaration):
            return {"kind": "mapped_declaration", "path": path, "line": line,
                    "declaration": declaration_fields(declaration)}
        case declarations.MappedNoDeclaration(path, line):
            return {"kind": "mapped_no_declaration", "path": path, "line": line}
        case declarations.UnmappedSite(reason):
            return {"kind": "unmapped_site", "reason": reason}


def proof_fields(proof: style_ledger.MappedSiteProof | style_ledger.GainedFileProof) -> dict[str, object]:
    match proof:
        case style_ledger.MappedSiteProof(site):
            return {"kind": "mapped_site_proof", "site": {
                "path": site.path, "line": site.line, "declaration": declaration_fields(site.declaration),
            }}
        case style_ledger.GainedFileProof(path, declaration):
            return {"kind": "gained_file_proof", "path": path, "declaration": declaration_fields(declaration)}


def outside_fields(outcome: Proof | style_ledger.NoOutsideEvidence) -> dict[str, object]:
    match outcome:
        case style_ledger.NoOutsideEvidence():
            return {"status": "not_applicable"}
        case style_ledger.OutsideRenameProven(declaration, path, line, proof):
            return {"status": "proven", "declaration": declaration_fields(declaration),
                    "path": path, "line": line, "proof": proof_fields(proof)}
        case style_ledger.OutsideRenameNotProven(mapped, candidates, reasons):
            examined: dict[str, object] = {"kind": "no_candidates"}
            if isinstance(candidates, style_ledger.OutsideCandidates):
                examined = {"kind": "examined_candidates", "items": [
                    {**asdict(item), "declaration": declaration_fields(item.declaration)}
                    for item in candidates.items
                ]}
            return {"status": "not_proven", "mapped": mapped_fields(mapped), "candidates": examined, "reasons": reasons}


def check_case(case: OutsideCase, relocation: dict[str, object],
               outside: Proof | style_ledger.NoOutsideEvidence) -> None:
    name, expected = case.finding.label, case.proof
    assert relocation["status"] == case.finding.status, (name, relocation)
    match outside:
        case style_ledger.NoOutsideEvidence():
            assert expected.outcome == "not_applicable", (name, outside)
        case style_ledger.OutsideRenameProven(line=line, proof=proof):
            assert proof_fields(proof)["kind"] == expected.outcome, (name, outside)
            assert not expected.line or line == expected.line, (name, outside)
        case style_ledger.OutsideRenameNotProven(mapped=site, reasons=reasons):
            assert expected.outcome == "not_proven" and reasons == expected.reasons, (name, outside)
            fields = mapped_fields(site)
            assert not expected.mapping or expected.mapping == fields.get("reason", fields["kind"]), (name, outside)


def read_answers(ledger: OutsideLedger, cases: dict[str, OutsideCase]) -> list[dict[str, object]]:
    tip = ledger._synced_tip()
    findings = {item.finding_id: item for item in ledger.held_findings(style_ledger.AllHeldFindings())}
    answers: list[dict[str, object]] = []
    keys = list(reversed(cases))
    for identity in (keys[0], UNKNOWN, *keys[1:], keys[0]):
        if identity == UNKNOWN:
            answers.append({"name": "unknown finding", "answer": {"id": identity, "status": "unknown_finding"}})
            continue
        case, finding = cases[identity], findings[identity]
        ledger.outside = style_ledger.NoOutsideEvidence()
        relocation = same_declaration(ledger, finding, tip)
        check_case(case, relocation, ledger.outside)
        anchor = ledger.finding_anchor(finding)
        anchor_fields = ({"status": "needs_judgment", "reason": anchor.reason} if isinstance(anchor, NeedsJudgment)
                         else {"status": "anchored", **asdict(anchor)})
        site = ledger._site_proof(finding, tip.commit)
        site_fields = ({"status": "unchanged", "site": {"path": site.path, "line": site.line}, "anchor": asdict(site.anchor)}
                       if isinstance(site, UnchangedSite) else {"status": "needs_judgment", "reason": site.reason})
        answer = {"id": identity, **anchor_fields, "original_site": site_fields,
                  "same_declaration": relocation, "outside_rename": outside_fields(ledger.outside)}
        answers.append({"name": case.finding.label, "answer": answer})
    return answers


def check_missing_source(repo: Path, root: Path, identity: str, blob: str) -> None:
    object_file = repo / ".git/objects" / blob[:2] / blob[2:]
    saved = object_file.read_bytes()
    object_file.unlink()
    try:
        with OutsideLedger(repo, root / "ledger.sqlite", access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
            try:
                same_declaration(ledger, ledger._held(identity), ledger._synced_tip())
            except style_ledger.LedgerError as error:
                assert "cat-file" in str(error) and blob in str(error), str(error)
                return
            raise AssertionError("Python accepted a missing candidate blob")
    finally:
        object_file.write_bytes(saved)


def main(root: Path) -> None:
    repo = root / "repo"
    fixture = OutsideFixture()
    fixture.case("missing_candidate", ORIGINAL, REMOVED, gained(),
                 prior_files={"b": "// Only the outside proof needs this old candidate file.\n"},
                 changed_files={"b": RENAMED})
    add_direct_cases(fixture)
    add_search_cases(fixture)
    add_map_cases(fixture)
    add_type_cases(fixture)
    add_child_cases(fixture)
    add_owner_cases(fixture)
    versions = fixture.history(repo)
    cases = {f"{index:016x}": case for index, case in enumerate(fixture.cases, 1)}
    write_ledger(repo, root, versions, {identity: case.finding for identity, case in cases.items()})
    database = root / "ledger.sqlite"
    before = database.read_bytes()
    with OutsideLedger(repo, database, access=style_ledger.LedgerAccess.READ_ONLY) as ledger:
        answers = read_answers(ledger, cases)
    missing_id = next(identity for identity, case in cases.items() if case.finding.label == "missing_candidate")
    missing_path = "crates/missing_candidate/src/b.rs"
    blob = git(repo, "rev-parse", f"{versions.read}:{missing_path}")
    check_missing_source(repo, root, missing_id, blob)
    assert database.read_bytes() == before, "The Python proof changed the ledger"
    print(json.dumps({"synced_tip": versions.tip, "cases": answers,
                      "missing_source": {"id": missing_id, "blob": blob, "read": versions.read,
                                         "path": missing_path}}, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
