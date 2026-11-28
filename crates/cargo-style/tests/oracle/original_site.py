"""Build private ledgers and obtain original-site answers from Python."""

from __future__ import annotations

import json
import shutil
import sqlite3
import subprocess
import sys
import tomllib
from contextlib import closing
from dataclasses import asdict, dataclass
from pathlib import Path

from scripts.style import rules
from scripts.style.finding_sites import FilingEligibleWithoutHistory, NeedsJudgment, UnchangedSite
from scripts.style.git_process import GitReadFailed
from scripts.style.style_config import ReaderSource
from scripts.style.style_ledger import (
    AllHeldFindings,
    CellInputScope,
    FindingAnchor,
    FindingKind,
    Ledger,
    LedgerAccess,
    ReadFiles,
    RepositoryTip,
    _fingerprint,
    _read_fingerprint,
    _tip,
)

STAMP = "2026-01-01T00:00:00+00:00"
OLDER_STAMP = "2025-12-31T00:00:00+00:00"
UNKNOWN = "f" * 16
MISSING_COMMIT = "f" * 40
SOURCE_DIR = "crates/alpha/src/"
UNCHANGED_CRATE = "é"
LINE = "    let old = 791;"
WORK = f"fn work() {{\n{LINE}\n    consume(old);\n}}\n"
HEADER = "// A new heading.\n"
CHANGED_BODY = HEADER + WORK.replace("consume(old)", "inspect(old)")


@dataclass(frozen=True)
class SourceChange:
    """A file's text at the judging commit and the synced tip."""

    name: str
    before: str
    after: str


@dataclass(frozen=True)
class SiteCase:
    """A stored finding and the strict site result the fixture must exercise."""

    name: str
    file: str
    expected: str | int
    rule: str = "names"
    state: str = "unfiled"
    text: str = LINE
    line: int = 2
    kind: FindingKind = "rename"
    subject: str = "old"
    history: str = "recorded"
    read_files: tuple[str, ...] = ()
    directory: str = SOURCE_DIR
    holder_paths: tuple[str, ...] = ()
    has_build_ticket: bool = False


def git(repo: Path, *arguments: str) -> str:
    result = subprocess.run(
        ("git", "-C", str(repo), *arguments), check=True, capture_output=True, text=True,
    )
    return result.stdout.strip()


def commit(repo: Path, title: str) -> str:
    git(repo, "add", "--all")
    git(repo, "commit", "-qm", title)
    return git(repo, "rev-parse", "HEAD")


def rules_at_base(repo: Path) -> None:
    """Write a private catalog with one rule for each family."""
    folder = repo / "docs/style"
    folder.mkdir(parents=True)
    for name, family, scope in (
        ("names", "names", "file"),
        ("values", "values", "file"),
        ("types", "types", "file"),
        ("modules", "modules", "file"),
        ("crate", "names", "crate"),
        ("inactive", "names", "file"),
        ("unreadable", "names", "file"),
        ("departed", "names", "file"),
    ):
        text = f"---\ntier: C\nfamily: {family}\ninput_scope: {scope}\n---\n\nUse the {name} fixture rule.\n"
        (folder / f"{name}.md").write_text(text, encoding="utf-8")
    (folder / "partial.md").write_text(
        "---\ntier: B\nfamily: types\ninput_scope: file\ncandidates:\n"
        "  kind: fields\n  surface: partial\n  uncovered: Other sites.\n---\n\nUse the fixture rule.\n",
        encoding="utf-8",
    )


def source_commits(repo: Path) -> tuple[str, str, str]:
    """Commit the judged source, a removal, and the current source."""
    for key, value in (
        ("user.name", "Original Site Test"),
        ("user.email", "site@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.precomposeUnicode", "true"),
        ("core.symlinks", "true"),
        ("maintenance.auto", "false"),
    ):
        git(repo, "config", key, value)
    (repo / SOURCE_DIR).mkdir(parents=True)
    (repo / "Cargo.toml").write_text(
        f'[workspace]\nmembers = ["crates/alpha", "{UNCHANGED_CRATE}"]\n', encoding="utf-8",
    )
    (repo / "crates/alpha/Cargo.toml").write_text(
        '[package]\nname = "alpha"\nversion = "0.1.0"\n', encoding="utf-8",
    )
    (repo / UNCHANGED_CRATE / "src").mkdir(parents=True)
    (repo / UNCHANGED_CRATE / "Cargo.toml").write_text(
        '[package]\nname = "unchanged"\nversion = "0.1.0"\n', encoding="utf-8",
    )
    (repo / UNCHANGED_CRATE / "src/lib.rs").write_text(WORK, encoding="utf-8")
    rules_at_base(repo)
    changes = (
        SourceChange("lib.rs", WORK, WORK),
        SourceChange("removed.rs", WORK, WORK),
        SourceChange("moved.rs", WORK, "fn sibling() {}\n" + WORK),
        SourceChange("body.rs", WORK, CHANGED_BODY),
        SourceChange("repeat.rs", WORK, WORK.replace(LINE, f"{LINE}\n{LINE}")),
        SourceChange("signature.rs", WORK, WORK.replace("work()", "work(input: usize)")),
        SourceChange("imports.rs", "use old_api::consume;\n" + WORK,
                     "use new_api::consume;\n" + WORK),
        SourceChange("unreadable.rs", WORK, HEADER + WORK + "/* unfinished\n"),
        SourceChange("a*.rs", WORK, HEADER + WORK),
        SourceChange("context.md", "Read this context.\n", "The context changed.\n"),
    )
    for change in changes:
        (repo / SOURCE_DIR / change.name).write_text(change.before, encoding="utf-8")
    (repo / SOURCE_DIR / "link.rs").symlink_to("lib.rs")
    auxiliary = repo / SOURCE_DIR / "aux.rs"
    auxiliary.write_text("// Original auxiliary text.\n", encoding="utf-8")
    base = commit(repo, "Judge the original source")
    (repo / SOURCE_DIR / "removed.rs").unlink()
    removal = commit(repo, "Remove the judged file")
    for change in changes:
        (repo / SOURCE_DIR / change.name).write_text(change.after, encoding="utf-8")
    auxiliary.write_bytes(b"// Auxiliary text with an invalid byte: \xff\n")
    names = repo / "docs/style/names.md"
    names.write_text(names.read_text(encoding="utf-8").replace("tier: C", "tier: C\ntags: [test]"),
                     encoding="utf-8")
    (repo / "docs/style/departed.md").unlink()
    return base, removal, commit(repo, "Advance the synced tip")


def site_cases() -> tuple[SiteCase, ...]:
    return (
        SiteCase("identical source with descriptive text", "lib.rs", 2, text="Describe the change."),
        SiteCase("line moves in the same names owner", "moved.rs", 3),
        SiteCase("filing eligibility cannot prove a site", "moved.rs", "no judging ticket for source anchor",
                 history="none"),
        SiteCase("closed finding", "lib.rs", "not an open Rust finding", state="waived"),
        SiteCase("non-Rust file", "context.md", "not an open Rust finding"),
        SiteCase("inactive judgment generation", "lib.rs", "judgment belongs to an inactive rule generation",
                 rule="inactive"),
        SiteCase("anchor refusal passes through", "lib.rs", "recorded anchor commit is missing",
                 history="missing"),
        SiteCase("follow has no original judgment", "moved.rs", "no original judgment commit",
                 history="orphan-follow", line=3),
        SiteCase("follow crosses the original removal", "removed.rs", "judgment crossed ancestry or removal boundary",
                 history="removed-follow"),
        SiteCase("symlink is not regular source", "link.rs", "not a regular source file"),
        SiteCase("rule left the catalog", "lib.rs", "source or input proof unreadable", rule="departed"),
        SiteCase("unreadable pinned rule", "lib.rs", "pinned rule family unreadable", rule="unreadable"),
        SiteCase("crate inputs changed", "lib.rs", "original context inputs changed or missing", rule="crate"),
        SiteCase("explicit read input changed", "lib.rs", "original context inputs changed or missing",
                 rule="types", read_files=(SOURCE_DIR + "context.md",)),
        SiteCase("old coordinate outside source", "moved.rs", "old source coordinate unreadable", line=200),
        SiteCase("stored text differs from source", "moved.rs", "finding text is not its old source line",
                 text="a description"),
        SiteCase("type subject is outside the original item", "moved.rs",
                 "proposed type subject is not located in its original item",
                 rule="types", kind="type", subject="AbsentType"),
        SiteCase("repeated line in the same owner", "repeat.rs", "unchanged source site not proved"),
        SiteCase("signature changed", "signature.rs", "unchanged source site not proved"),
        SiteCase("imports changed around a type", "imports.rs", "enclosing item, owner, imports or original inputs changed",
                 rule="types", line=3),
        SiteCase("unreadable enclosing syntax", "unreadable.rs", "source or input proof unreadable"),
        SiteCase("body changes around a name", "body.rs", 3),
        SiteCase("body changes around a value", "body.rs", 3, rule="values", kind="restructure"),
        SiteCase("body changes around a type", "body.rs", "enclosing item, owner, imports or original inputs changed",
                 rule="types", kind="type"),
        SiteCase("body changes around a module finding", "body.rs", "enclosing item, owner, imports or original inputs changed",
                 rule="modules", kind="restructure"),
        SiteCase("types keep an identical item", "moved.rs", 3, rule="types", kind="type"),
        SiteCase("modules keep an identical item", "moved.rs", 3, rule="modules", kind="restructure"),
        SiteCase("uncovered component uses its base rule", "moved.rs", 3, rule="partial#uncovered", kind="type"),
        SiteCase("newer holder at equal character length", "lib.rs", "original context inputs changed or missing",
                 rule="crate", directory=f"{UNCHANGED_CRATE}/src/", holder_paths=(UNCHANGED_CRATE, ".")),
        SiteCase("unchanged crate inputs", "lib.rs", 2, rule="crate",
                 directory=f"{UNCHANGED_CRATE}/src/", holder_paths=(UNCHANGED_CRATE,)),
        SiteCase("unchanged crate inputs with build history", "lib.rs", 2, rule="crate",
                 directory=f"{UNCHANGED_CRATE}/src/", holder_paths=(UNCHANGED_CRATE,), has_build_ticket=True),
        SiteCase("invalid UTF-8 patch from another glob match", "a*.rs", "source or input proof unreadable"),
    )


def rule_id(name: str) -> str:
    base, marker, component = name.partition("#")
    return f"local:docs/style/{base}.md{marker}{component}"


def insert_holder(
    db: sqlite3.Connection, repo: Path, site: SiteCase, path: str, rule: str, pin: str, ticket: str, original: RepositoryTip,
) -> None:
    scope = CellInputScope.CRATE if site.rule == "crate" else CellInputScope.FILE
    holder_paths = site.holder_paths or (path,)
    for index, holder_path in enumerate(holder_paths, 1):
        fingerprint = (_read_fingerprint(original, holder_path, ReadFiles(site.read_files)) if site.read_files
                       else _fingerprint(repo, original, scope, holder_path))
        db.execute(
            'INSERT INTO judgments (rule,rule_blob,input_scope,input_fingerprint,path,"commit",verdict,verdict_detail,'
            "ticket,worker,recorded_at,declared_scope,read_files,generation) "
            "VALUES (?,?,?,?,?,?,'held','',?,'test',?,'rule',?,?)",
            (rule, pin, scope.value, fingerprint, holder_path, original.commit, ticket,
             STAMP if index == len(holder_paths) else OLDER_STAMP,
             json.dumps(site.read_files or None), -1 if site.rule == "inactive" else 0),
        )


def insert_finding(
    db: sqlite3.Connection, repo: Path, identity: str, site: SiteCase, original: RepositoryTip, removal: str, tip: str,
) -> None:
    base = original.commit
    path = site.directory + site.file
    rule = rule_id(site.rule)
    rule_path = rule.split(":", 1)[1].partition("#")[0]
    pin = git(repo, "rev-parse", f"{base}:{rule_path}")
    if site.rule == "unreadable":
        pinned = b"family: names\n\xff"
    else:
        pinned = subprocess.run(
            ("git", "-C", str(repo), "show", f"{base}:{rule_path}"), check=True, capture_output=True,
        ).stdout
    db.execute("INSERT OR IGNORE INTO rule_texts (blob,text) VALUES (?,?)", (pin, pinned))
    ticket = f"read-{identity}"
    anchor = {"recorded": base, "removed-follow": base, "missing": MISSING_COMMIT}.get(site.history, "")
    follow = tip if site.history in {"orphan-follow", "removed-follow"} else ""
    db.execute(
        "INSERT INTO held_findings (finding_id,rule,path,line,text,kind,before,after,rule_blob,ticket,state,proposal,"
        "created_at,updated_at,anchor_commit,followed_from,followed_from_line,followed_at,followed_from_commit) "
        "VALUES (?,?,?,?,?,?,?,?,?,?,?,'',?,?,?,?,?,?,?)",
        (identity, rule, path, site.line, site.text, site.kind, site.subject, "fresh", pin, ticket, site.state,
         STAMP, STAMP, anchor, path if follow else "", site.line if follow else 0, follow, base if follow else ""),
    )
    if site.name == "identical source with descriptive text":
        current = git(repo, "rev-parse", f"{tip}:{rule_path}")
        insert_holder(db, repo, site, path, rule, current, ticket, original)
    if site.rule in {"types", "modules", "inactive", "crate", "partial#uncovered", "departed"}:
        insert_holder(db, repo, site, path, rule, pin, ticket, original)
    if site.has_build_ticket:
        build_ticket = f"build-{identity}"
        db.execute(
            "INSERT INTO tickets (id,unit,kind,family,rule,crate,files,sites,rules,rule_blobs,state,branch,"
            "checkpoint,base,created_at,updated_at,finding_ids) "
            "VALUES (?,?,'build','names',?,'unchanged','[]','[]','[]','{}','merged','main',?,?,?,?,?)",
            (build_ticket, build_ticket, rule, tip, base, STAMP, STAMP, json.dumps([identity])),
        )
        db.execute("UPDATE held_findings SET build_ticket=? WHERE finding_id=?", (build_ticket, identity))
    if site.history == "removed-follow":
        db.execute(
            'INSERT INTO judgments (rule,rule_blob,input_scope,input_fingerprint,path,"commit",verdict,verdict_detail,'
            "ticket,worker,recorded_at) VALUES (?,?,'file','',?,?,'removed','','removal','test',?)",
            (rule, pin, path, removal, STAMP),
        )


def build_ledger(repo: Path, database: Path, cases: dict[str, SiteCase], base: str, removal: str, tip: str) -> None:
    """Create the schema with Ledger and insert one row per fixture finding."""
    with Ledger(repo, database):
        pass
    original = _tip(repo, base, reader_source=ReaderSource.PYTHON)
    with closing(sqlite3.connect(database)) as db, db:
        db.execute("INSERT INTO sync (id,tip,synced_at) VALUES (1,?,?)", (tip, STAMP))
        for identity, site in cases.items():
            insert_finding(db, repo, identity, site, original, removal, tip)


def write_catalog(repo: Path, base: str, root: Path) -> str:
    """Export actual catalog data and one negative fixture with a missing pin."""
    catalog = {}
    history = rules.ValidatedRuleHistory(repo)
    try:
        for rule in rules.load_rules(repo):
            pin = git(repo, "rev-parse", f"{base}:{rule.path.relative_to(repo).as_posix()}")
            catalog[rule.rule_id] = {
                "tier": rule.tier,
                "family": rule.family,
                "surface": rule.candidates.surface if rule.candidates else None,
                "blob": rule.blob,
                "identities": {pin: history.identity(pin, rule) or None, rule.blob: rule.judgment_identity or None},
            }
    finally:
        history.close()
    (root / "rules.json").write_text(json.dumps({"rules": catalog}, sort_keys=True), encoding="utf-8")
    names = catalog[rule_id("names")]
    missing, = set(names["identities"]) - {names["blob"]}
    del names["identities"][missing]
    (root / "missing-rule-blob.json").write_text(json.dumps({"rules": catalog}, sort_keys=True), encoding="utf-8")
    return missing


def anchor_fields(anchor: FindingAnchor | NeedsJudgment) -> dict[str, object]:
    if isinstance(anchor, NeedsJudgment):
        return {"status": "needs_judgment", "reason": anchor.reason}
    return {"status": "anchored", **asdict(anchor)}


def site_fields(proof: UnchangedSite | NeedsJudgment) -> dict[str, object]:
    if isinstance(proof, NeedsJudgment):
        return {"status": "needs_judgment", "reason": proof.reason}
    return {"status": "unchanged", "site": {"path": proof.path, "line": proof.line}, "anchor": asdict(proof.anchor)}


def read_answers(repo: Path, database: Path, cases: dict[str, SiteCase], tip: str) -> list[dict[str, object]]:
    """Check each fixture reaches its intended branch and export the reply."""
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        findings = {finding.finding_id: finding for finding in ledger.held_findings(AllHeldFindings())}
        answers: list[dict[str, object]] = []
        keys = list(reversed(cases))
        for identity in (keys[0], UNKNOWN, *keys[1:], keys[0]):
            if identity == UNKNOWN:
                answers.append({"name": "unknown finding", "answer": {"id": identity, "status": "unknown_finding"}})
                continue
            site = cases[identity]
            finding = findings[identity]
            anchor = ledger.finding_anchor(finding)
            proof = ledger._site_proof(finding, tip)
            if isinstance(site.expected, int):
                assert isinstance(proof, UnchangedSite), (site.name, proof)
                assert (proof.path, proof.line) == (finding.path, site.expected), (site.name, proof)
            else:
                assert proof == NeedsJudgment(site.expected), (site.name, proof)
            if site.history == "none":
                assert ledger.finding_site(finding, tip) == FilingEligibleWithoutHistory(finding.path, finding.line)
            answers.append({"name": site.name,
                            "answer": {"id": identity, **anchor_fields(anchor), "original_site": site_fields(proof)}})
    return answers


@dataclass(frozen=True)
class FailedSiteRequest:
    """A request that must print one diagnostic and no reply."""

    name: str
    ledger: str
    finding: str
    diagnostic: str


def read_site(repo: Path, database: Path, finding: str, tip: str) -> UnchangedSite | NeedsJudgment:
    with Ledger(repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        return ledger._site_proof(ledger._held(finding), tip)


def invalid_cell_history(
    repo: Path, database: Path, root: Path, cases: dict[str, SiteCase], tip: str,
) -> FailedSiteRequest:
    """Keep the removal active and place the finding after it."""
    identity, = (identity for identity, site in cases.items() if site.history == "removed-follow")
    target = root / "invalid-cell-history.sqlite"
    shutil.copyfile(database, target)
    with closing(sqlite3.connect(target)) as db, db:
        db.execute("UPDATE held_findings SET anchor_commit=?, followed_at='' WHERE finding_id=?", (tip, identity))
    assert isinstance(read_site(repo, target, identity, tip), UnchangedSite)
    with closing(sqlite3.connect(target)) as db, db:
        db.execute(
            'INSERT INTO judgments (rule,rule_blob,input_scope,input_fingerprint,path,"commit",verdict,verdict_detail,'
            "ticket,worker,recorded_at) SELECT rule,rule_blob,'file','',path,?,'clean','','other-read','test',? "
            "FROM held_findings WHERE finding_id=?",
            (MISSING_COMMIT, STAMP, identity),
        )
    try:
        result = read_site(repo, target, identity, tip)
    except GitReadFailed as error:
        assert "merge-base" in str(error) and MISSING_COMMIT in str(error), str(error)
        return FailedSiteRequest("unresolvable judgment after a removal", target.name, identity, MISSING_COMMIT)
    raise AssertionError(f"Python returned a reply for the unresolvable judgment: {result}")


def invalid_synced_tip(repo: Path, database: Path, root: Path, finding: str) -> FailedSiteRequest:
    """Commit malformed TOML and use it as the synced tip."""
    target = root / "invalid-synced-tip.sqlite"
    shutil.copyfile(database, target)
    config = "rustfmt.toml"
    (repo / config).write_text("ignore = [\n", encoding="utf-8")
    bad_tip = commit(repo, "Sync invalid Rust format settings")
    with closing(sqlite3.connect(target)) as db, db:
        db.execute("UPDATE sync SET tip=? WHERE id=1", (bad_tip,))
    try:
        result = read_site(repo, target, finding, bad_tip)
    except tomllib.TOMLDecodeError:
        return FailedSiteRequest("malformed committed TOML", target.name, finding, config)
    raise AssertionError(f"Python returned a reply for the malformed synced tip: {result}")


def undecodable_synced_tree(
    repo: Path, database: Path, root: Path, finding: str, tip: str,
) -> FailedSiteRequest:
    """Add a raw path to a tree without creating it on disk."""
    target = root / "undecodable-synced-tree.sqlite"
    shutil.copyfile(database, target)
    entries = subprocess.run(
        ("git", "-C", str(repo), "ls-tree", "-z", tip), check=True, capture_output=True,
    ).stdout
    blob = git(repo, "rev-parse", f"{tip}:Cargo.toml")
    entry = b"100644 blob " + blob.encode() + b"\tinvalid-\xff.rs\0"
    tree = subprocess.run(
        ("git", "-C", str(repo), "mktree", "-z"), input=entries + entry, check=True, capture_output=True,
    ).stdout.decode().strip()
    bad_tip = git(repo, "commit-tree", tree, "-p", tip, "-m", "Add an undecodable path")
    with closing(sqlite3.connect(target)) as db, db:
        db.execute("UPDATE sync SET tip=? WHERE id=1", (bad_tip,))
    try:
        result = read_site(repo, target, finding, bad_tip)
    except UnicodeDecodeError as error:
        assert b"invalid-\xff.rs\0" in error.object, str(error)
        return FailedSiteRequest("undecodable synced tree", target.name, finding, "UTF-8")
    raise AssertionError(f"Python returned a reply for the undecodable synced tree: {result}")


def non_utf8_ignore_case(repo: Path, database: Path, root: Path, finding: str, tip: str) -> dict[str, object]:
    expected = read_site(repo, database, finding, tip)
    assert isinstance(expected, UnchangedSite), expected
    git(repo, "switch", "-q", "--detach", tip)
    (repo / "rustfmt.toml").write_bytes(b"ignore = []\n# Invalid UTF-8: \xff\n")
    synced = commit(repo, "Save an invalid UTF-8 ignore comment")
    git(repo, "switch", "-q", "main")
    target = root / "non-utf8-ignore.sqlite"
    shutil.copyfile(database, target)
    with closing(sqlite3.connect(target)) as db, db:
        db.execute("UPDATE sync SET tip=? WHERE id=1", (synced,))
    with Ledger(repo, target, access=LedgerAccess.READ_ONLY) as ledger:
        held = ledger._held(finding)
        proof = ledger._site_proof(held, synced)
        assert proof == expected, proof
        anchor = ledger.finding_anchor(held)
    return {"ledger": target.name, "synced_tip": synced,
            "site": {"name": "finding preserves the site with a non-UTF-8 ignore comment",
                     "answer": {"id": finding, **anchor_fields(anchor), "original_site": site_fields(expected)}}}


def main(root: Path) -> None:
    repo = root / "repo"
    base, removal, tip = source_commits(repo)
    database = root / "ledger.sqlite"
    cases = {f"{index:016x}": case for index, case in enumerate(site_cases(), 1)}
    build_ledger(repo, database, cases, base, removal, tip)
    missing = write_catalog(repo, base, root)
    answers = read_answers(repo, database, cases, tip)
    good_finding = next(iter(cases))
    failures = (invalid_cell_history(repo, database, root, cases, tip),
                invalid_synced_tip(repo, database, root, good_finding),
                undecodable_synced_tree(repo, database, root, good_finding, tip))
    non_utf8_ignore = non_utf8_ignore_case(repo, database, root, good_finding, tip)
    print(json.dumps({"synced_tip": tip, "missing_pin": missing, "missing_rule": rule_id("names"),
                      "non_utf8_ignore": non_utf8_ignore,
                      "missing_pin_finding": good_finding, "cases": answers,
                      "failures": [asdict(failure) for failure in failures]}, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
