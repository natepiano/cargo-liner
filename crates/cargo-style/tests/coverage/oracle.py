"""Build private coverage fixtures and read the expected cells and counts."""

from contextlib import closing
from dataclasses import dataclass
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
from typing import Literal, NotRequired, TypedDict

from scripts.style import rules
from scripts.style.git_process import GitReadFailed, LedgerError
from scripts.style.style_config import ReaderSource
from scripts.style.style_ledger import (
    CellInputScope,
    CoverageScope,
    CrateCoverage,
    Ledger,
    LedgerAccess,
    MIGRATIONS,
    ReadFiles,
    WholeRepository,
    _config_hash,  # pyright: ignore[reportPrivateUsage]
    _fingerprint,  # pyright: ignore[reportPrivateUsage]
    _read_fingerprint,  # pyright: ignore[reportPrivateUsage]
    _tip,  # pyright: ignore[reportPrivateUsage]
)

ALPHA = "crates/alpha/src/"
BETA = "crates/beta/src/"
CODE = "fn work() {}\n"
STAMP = "2026-01-01T00:00:00Z"
COMPATIBLE_SCHEMA = 47
MISSING_REMOVAL_COMMIT = "e" * 40
RULES_CATALOG = "rules.json"
SITES_CATALOG = "sites.json"
CELLS = "--cells"
RULE = "--rule"
JUDGMENT_INSERT = (
    'INSERT INTO judgments (rule,rule_blob,input_scope,input_fingerprint,path,"commit",verdict,'
    "verdict_detail,ticket,worker,recorded_at,declared_scope,read_files,generation) "
    "VALUES (?,?,?,?,?,?,?,'','read','fixture',?,?,?,?)"
)

type SqlParameter = str | int


@dataclass(frozen=True)
class Revisions:
    base: str
    removed: str
    synced: str
    branch: str


@dataclass(frozen=True)
class Judgment:
    rule: str
    path: str
    commit: str
    verdict: str = "clean"
    scope: CellInputScope = CellInputScope.FILE
    declared: str = "rule"
    reads: tuple[str, ...] = ()
    generation: int = 0


class RuleDefinition(TypedDict):
    tier: str
    family: str | None
    blob: str
    input_scope: str
    reach: str | None
    surface: str | None
    mechanism: str | None
    identities: dict[str, str | None]


type Catalog = dict[str, RuleDefinition]


class CellRow(TypedDict):
    rule: str
    path: str
    crate: str
    tier: str
    family: str | None
    state: str


class StateCounts(TypedDict):
    current: int
    stale: int
    open: int
    awaiting_merge: int
    held: int


class RuleCountRow(StateCounts):
    rule_id: str
    tier: str
    input_scope: str


class FamilyRow(StateCounts):
    family: str | None


class AvailableCoverage(TypedDict):
    status: Literal["available"]
    cells: NotRequired[list[CellRow]]
    rules: list[RuleCountRow]
    families: list[FamilyRow]


class UnavailableCoverage(TypedDict):
    status: Literal["unavailable"]
    prerequisite: Literal["candidate_sites"]
    rules: list[str]


class Observed(TypedDict):
    synced_tip: str
    ledger_revision: Literal["absent"]


class ReplyBody(TypedDict):
    schema: int
    observed: Observed
    coverage: AvailableCoverage | UnavailableCoverage


class ReplyResult(TypedDict):
    kind: Literal["reply"]
    body: ReplyBody


class RefusedResult(TypedDict):
    kind: Literal["refused"]
    diagnostic: str


class CoverageCase(TypedDict):
    name: str
    ledger: str
    rules: str
    flags: list[str]
    result: ReplyResult | RefusedResult


@dataclass(frozen=True)
class Fixture:
    root: Path
    repo: Path
    database: Path
    definitions: Catalog
    revisions: Revisions


def git(repo: Path, *arguments: str) -> str:
    return subprocess.run(
        ("git", "-C", str(repo), *arguments),
        check=True,
        text=True,
        capture_output=True,
    ).stdout.strip()


def write(repo: Path, path: str, text: str) -> None:
    destination = repo / path
    destination.parent.mkdir(parents=True, exist_ok=True)
    _ = destination.write_text(text, encoding="utf-8")


def commit(repo: Path, message: str) -> str:
    _ = git(repo, "add", "--all")
    _ = git(repo, "commit", "-qm", message)
    return git(repo, "rev-parse", "HEAD")


def install_rules(repo: Path) -> None:
    definitions = {
        "docs/style/names.md": "tier: C\nfamily: names\ninput_scope: file",
        "docs/style/crate.md": "tier: C\nfamily: types\ninput_scope: crate",
        "docs/style/manifest.md": "tier: C\nfamily: values\ninput_scope: manifest",
        "docs/style/modules.md": "tier: C\nfamily: modules\ninput_scope: file",
        "docs/style/lint.md": "tier: A\nmechanism: clippy",
        "docs/style/lint-stale.md": "tier: A\nmechanism: rustfmt",
        "docs/style/lint-open.md": "tier: A\nmechanism: mend",
        "docs/style/manual.md": "tier: E",
        "docs/alpha/style/member.md": "tier: C\nfamily: names\ninput_scope: file",
    }
    for path, metadata in definitions.items():
        write(repo, path, f"---\n{metadata}\n---\n\nUse the fixture rule in {path}.\n")


def repository(root: Path) -> tuple[Path, Revisions]:
    repo = root / "repo"
    repo.mkdir()
    _ = git(repo, "init", "-q", "--object-format=sha1", "--initial-branch=main")
    for key, value in (
        ("user.name", "Coverage fixture"),
        ("user.email", "coverage@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.precomposeUnicode", "true"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)
    workspace = '[workspace]\nmembers = ["crates/alpha", "crates/beta"]\n'
    write(repo, "Cargo.toml", workspace)
    for name in ("alpha", "beta"):
        write(repo, f"crates/{name}/Cargo.toml", f'[package]\nname = "{name}"\nversion = "0.1.0"\n')
    install_rules(repo)
    for file in ("lib", "stale", "branch", "removed", "branch-removed", "inactive", "missing", "explicit", "comment", "inner"):
        write(repo, f"{ALPHA}{file}.rs", CODE)
    write(repo, f"{BETA}lib.rs", CODE)
    write(repo, "réserve.rs", CODE)
    write(repo, "context.md", "Original context.\n")
    base = commit(repo, "Record coverage inputs")
    (repo / f"{ALPHA}removed.rs").unlink()
    removed = commit(repo, "Remove a judged file")
    write(repo, f"{ALPHA}removed.rs", CODE)
    write(repo, f"{ALPHA}stale.rs", CODE.replace("work", "changed"))
    write(repo, f"{ALPHA}comment.rs", "// A regular comment.\n" + CODE)
    write(repo, f"{ALPHA}inner.rs", "//! An inner doc comment.\n" + CODE)
    write(repo, "context.md", "Changed context.\n")
    write(repo, "rustfmt.toml", "max_width = 95\n")
    write(repo, "Cargo.toml", workspace + '\n[workspace.lints.rust]\nunsafe_code = "deny"\n')
    names = "docs/style/names.md"
    write(repo, names, (repo / names).read_text(encoding="utf-8").replace("tier: C", "tier: C\ntags: [test]"))
    synced = commit(repo, "Advance the synced tip")
    _ = git(repo, "switch", "-qc", "style/main")
    write(repo, f"{ALPHA}branch.rs", CODE.replace("work", "on_branch"))
    (repo / f"{ALPHA}branch-removed.rs").unlink()
    branch = commit(repo, "Judge changes on the style branch")
    _ = git(repo, "switch", "-q", "main")
    return repo, Revisions(base, removed, synced, branch)


def judgments(revisions: Revisions) -> tuple[Judgment, ...]:
    base, removed, branch = revisions.base, revisions.removed, revisions.branch
    return (
        Judgment("names", ALPHA + "lib.rs", base, verdict="held"),
        Judgment("names", ALPHA + "lib.rs", branch, verdict="exception"),
        Judgment("names", ALPHA + "stale.rs", base),
        Judgment("names", ALPHA + "branch.rs", branch),
        Judgment("names", ALPHA + "removed.rs", base),
        Judgment("names", ALPHA + "removed.rs", removed, verdict="removed"),
        Judgment("names", ALPHA + "branch-removed.rs", branch, verdict="removed"),
        Judgment("names", ALPHA + "inactive.rs", base, generation=-1),
        Judgment("names", ALPHA + "missing.rs", "f" * 40),
        Judgment("names", ALPHA + "explicit.rs", base, reads=("context.md",)),
        Judgment("crate", ALPHA + "lib.rs", base, scope=CellInputScope.CRATE),
        Judgment("crate", BETA + "lib.rs", base, scope=CellInputScope.CRATE),
        Judgment("crate", "réserve.rs", base, scope=CellInputScope.CRATE),
        Judgment("crate", ALPHA + "explicit.rs", base, declared="file"),
        Judgment("manifest", ALPHA + "lib.rs", base, scope=CellInputScope.MANIFEST),
        Judgment("manifest", BETA + "lib.rs", revisions.synced, scope=CellInputScope.MANIFEST),
        Judgment("modules", ALPHA + "comment.rs", base),
        Judgment("modules", ALPHA + "inner.rs", base),
    )


def rule_id(name: str) -> str:
    return f"local:docs/style/{name}.md"


def pin_at(repo: Path, commit_id: str, name: str) -> str:
    return git(repo, "rev-parse", f"{commit_id}:docs/style/{name}.md")


def build_ledger(repo: Path, path: Path, revisions: Revisions, version: int) -> None:
    with closing(sqlite3.connect(path)) as connection, connection:
        _ = connection.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)")
        for number, migration in sorted(MIGRATIONS.items()):
            if number <= version:
                _ = connection.executescript(migration)
        _ = connection.execute("INSERT INTO schema_version VALUES (?)", (version,))
        _ = connection.execute(
            "INSERT INTO sync(id,tip,style_branch,synced_at) VALUES (1,?,?,?)",
            (revisions.synced, "refs/heads/style/main", STAMP),
        )
        insert_judgments(repo, connection, revisions)
        insert_lint(repo, connection, revisions)


def judgment_row(repo: Path, revisions: Revisions, judgment: Judgment, fingerprint: str) -> tuple[SqlParameter, ...]:
    return (rule_id(judgment.rule), pin_at(repo, revisions.base, judgment.rule), judgment.scope.value,
            fingerprint, judgment.path, judgment.commit, judgment.verdict, STAMP, judgment.declared,
            json.dumps(judgment.reads or None), judgment.generation)


def insert_judgments(repo: Path, connection: sqlite3.Connection, revisions: Revisions) -> None:
    tips = {commit_id: _tip(repo, commit_id, reader_source=ReaderSource.PYTHON)
            for commit_id in (revisions.base, revisions.branch, revisions.synced)}
    for judgment in judgments(revisions):
        source = tips.get(judgment.commit)
        fingerprint = ""
        if source is not None and judgment.verdict != "removed":
            fingerprint = (_read_fingerprint(source, judgment.path, ReadFiles(judgment.reads)) if judgment.reads
                           else _fingerprint(repo, source, judgment.scope, judgment.path))
        _ = connection.execute(JUDGMENT_INSERT, judgment_row(repo, revisions, judgment, fingerprint))


def insert_lint(repo: Path, connection: sqlite3.Connection, revisions: Revisions) -> None:
    tip = _tip(repo, revisions.synced, reader_source=ReaderSource.PYTHON)
    base = _tip(repo, revisions.base, reader_source=ReaderSource.PYTHON)
    for tool, config, name in (
        ("clippy", _config_hash(repo, tip, "clippy"), "lint"),
        ("rustfmt", _config_hash(repo, base, "rustfmt"), "lint-stale"),
    ):
        _ = connection.execute(
            "INSERT INTO lint_runs(tip,tree,tool,scope,config_hash,passed,diagnostics_json,rule_blobs_json) "
            + "VALUES (?,?,?,'workspace',?,1,?,?)",
            (tip.commit, tip.tree, tool, config, json.dumps({ALPHA + "stale.rs": 1}),
             json.dumps({rule_id(name): pin_at(repo, revisions.base, name)})),
        )


def catalog(repo: Path, revisions: Revisions) -> Catalog:
    history = rules.ValidatedRuleHistory(repo)
    mechanisms = {"lint": "clippy", "lint-stale": "rustfmt", "lint-open": "mend"}
    try:
        result: Catalog = {}
        for rule in rules.load_rules(repo):
            old = git(repo, "rev-parse", f"{revisions.base}:{rule.path.relative_to(repo).as_posix()}")
            reach = rule.reach.crate_dir.as_posix() if isinstance(rule.reach, rules.MemberReach) else None
            result[rule.rule_id] = RuleDefinition(
                tier=rule.tier, family=rule.family, blob=rule.blob, input_scope=rule.input_scope, reach=reach,
                surface=rule.candidates.surface if rule.candidates else None,
                mechanism=mechanisms.get(rule.name),
                identities={old: history.identity(old, rule) or None, rule.blob: rule.judgment_identity or None},
            )
        return result
    finally:
        history.close()


def write_catalog(root: Path, filename: str, definitions: Catalog) -> None:
    write(root, filename, json.dumps({"rules": definitions}, sort_keys=True))


def family_rows(counts: list[RuleCountRow], definitions: Catalog) -> list[FamilyRow]:
    families: dict[str | None, FamilyRow] = {}
    for row in counts:
        family = definitions[row["rule_id"]]["family"]
        total = families.setdefault(
            family, FamilyRow(family=family, current=0, stale=0, open=0, awaiting_merge=0, held=0))
        total["current"] += row["current"]
        total["stale"] += row["stale"]
        total["open"] += row["open"]
        total["awaiting_merge"] += row["awaiting_merge"]
        total["held"] += row["held"]
    return [families[family] for family in sorted(families, key=lambda family: family or "")]


def coverage(fixture: Fixture, database: Path, crate: str | None = None, rule: str | None = None) -> AvailableCoverage:
    """Python `Ledger.cells` and `Ledger.rule_counts`, with the rows of `rule` only when a rule is named."""
    scope: CoverageScope = CrateCoverage(crate) if crate else WholeRepository()
    with Ledger(fixture.repo, database, access=LedgerAccess.READ_ONLY) as ledger:
        cells = sorted((CellRow(rule=cell.rule, path=cell.path, crate=cell.crate, tier=cell.tier, family=cell.family,
                                state=cell.state) for cell in ledger.cells(rule=rule, crate=crate)),
                       key=lambda row: (row["rule"], row["path"]))
        counts = sorted((RuleCountRow(rule_id=row.rule_id, tier=row.tier, input_scope=row.input_scope,
                                      current=row.current, stale=row.stale, open=row.open,
                                      awaiting_merge=row.awaiting_merge, held=row.held)
                         for row in ledger.rule_counts(scope) if rule is None or row.rule_id == rule),
                        key=lambda row: row["rule_id"])
    return AvailableCoverage(status="available", cells=cells, rules=counts,
                             families=family_rows(counts, fixture.definitions))


def unavailable(rule_ids: list[str]) -> UnavailableCoverage:
    return UnavailableCoverage(status="unavailable", prerequisite="candidate_sites", rules=rule_ids)


def reply(name: str, ledger: Path, catalog_name: str, flags: list[str], tip: str,
          answer: AvailableCoverage | UnavailableCoverage) -> CoverageCase:
    body = ReplyBody(schema=1, observed=Observed(synced_tip=tip, ledger_revision="absent"), coverage=answer)
    return CoverageCase(name=name, ledger=ledger.name, rules=catalog_name, flags=flags,
                        result=ReplyResult(kind="reply", body=body))


def refused(name: str, ledger: Path, flags: list[str], diagnostic: str) -> CoverageCase:
    return CoverageCase(name=name, ledger=ledger.name, rules=RULES_CATALOG, flags=flags,
                        result=RefusedResult(kind="refused", diagnostic=diagnostic))


def copy_ledger(database: Path, destination: Path, statement: str, parameters: tuple[SqlParameter, ...] = ()) -> None:
    with closing(sqlite3.connect(database)) as original, closing(sqlite3.connect(destination)) as copy:
        original.backup(copy)
        with copy:
            _ = copy.execute(statement, parameters)


def malformed_history_case(fixture: Fixture) -> CoverageCase:
    malformed = fixture.root / "malformed.sqlite"
    copy_ledger(fixture.database, malformed, "UPDATE judgments SET read_files='not JSON' WHERE read_files!='null'")
    try:
        _ = coverage(fixture, malformed)
    except json.JSONDecodeError:
        return refused("malformed judgment history", malformed, [CELLS], "read_files")
    raise AssertionError("The Python ledger accepted malformed read_files")


def failed_git_read_case(fixture: Fixture) -> CoverageCase:
    # This commit resolves, but its tree cannot supply branch coverage.
    broken_commit = (
        f"tree {'f' * 40}\nparent {fixture.revisions.synced}\n"
        + "author Coverage <coverage@example.invalid> 1767225600 +0000\n"
        + "committer Coverage <coverage@example.invalid> 1767225600 +0000\n\nMissing tree\n"
    )
    stored = subprocess.run(
        ("git", "-C", str(fixture.repo), "hash-object", "--literally", "-t", "commit", "-w", "--stdin"),
        input=broken_commit, check=True, capture_output=True, text=True,
    ).stdout.strip()
    damaged_tree = fixture.root / "damaged-tree.sqlite"
    copy_ledger(fixture.database, damaged_tree, "UPDATE sync SET style_branch=? WHERE id=1", (stored,))
    try:
        _ = coverage(fixture, damaged_tree)
    except GitReadFailed:
        return refused("failed Git read after a resolved commit", damaged_tree, [CELLS],
                       f"cargo-style: cannot read the tree of commit {stored}: ")
    raise AssertionError("The Python ledger read the missing tree")


def missing_removal_commit_case(fixture: Fixture) -> CoverageCase:
    missing_removal = fixture.root / "missing-removal.sqlite"
    removal = Judgment("names", ALPHA + "lib.rs", MISSING_REMOVAL_COMMIT, verdict="removed")
    copy_ledger(fixture.database, missing_removal, JUDGMENT_INSERT,
                judgment_row(fixture.repo, fixture.revisions, removal, ""))
    try:
        _ = coverage(fixture, missing_removal)
    except GitReadFailed:
        return refused("removal row at a missing commit", missing_removal, [CELLS],
                       f"cargo-style: cannot resolve the removal commit {MISSING_REMOVAL_COMMIT}")
    raise AssertionError("The Python ledger read a removal at a missing commit")


def unknown_rule_case(fixture: Fixture) -> CoverageCase:
    unknown = rule_id("absent")
    message = f"unknown rule {unknown}"
    try:
        _ = coverage(fixture, fixture.database, rule=unknown)
    except LedgerError as error:
        if str(error) != message:
            raise
        return refused("unknown rule", fixture.database, [RULE, unknown], f"cargo-style: {message}")
    raise AssertionError("The Python ledger accepted an unknown rule")


def refused_cases(fixture: Fixture) -> list[CoverageCase]:
    return [malformed_history_case(fixture), failed_git_read_case(fixture), missing_removal_commit_case(fixture),
            unknown_rule_case(fixture)]


def sites_cases(fixture: Fixture, latest: Path) -> list[CoverageCase]:
    """Tier B and D rules need candidate sites, so each request that selects one is unavailable."""
    sites = dict(fixture.definitions)
    site_rules: list[str] = []
    cases: list[CoverageCase] = []
    synced = fixture.revisions.synced
    for tier, family in (("B", "types"), ("D", "modules")):
        name = rule_id(f"sites-{tier.lower()}")
        definition = fixture.definitions[rule_id("manifest")].copy()
        definition["tier"], definition["family"], definition["surface"] = tier, family, "whole"
        sites[name] = definition
        site_rules.append(name)
        cases.append(reply(f"tier {tier} requires sites", latest, SITES_CATALOG, [RULE, name], synced,
                           unavailable([name])))
    write_catalog(fixture.root, SITES_CATALOG, sites)
    cases.append(reply("all rules with tier B and D rules require sites", latest, SITES_CATALOG, [CELLS], synced,
                       unavailable(sorted(site_rules))))
    return cases


def main(root: Path) -> None:
    repo, revisions = repository(root)
    latest = max(MIGRATIONS)
    for version in (latest, COMPATIBLE_SCHEMA):
        build_ledger(repo, root / f"schema-{version}.sqlite", revisions, version)
    database = root / f"schema-{latest}.sqlite"
    compatible = root / f"schema-{COMPATIBLE_SCHEMA}.sqlite"
    fixture = Fixture(root, repo, database, catalog(repo, revisions), revisions)
    write_catalog(root, RULES_CATALOG, fixture.definitions)
    whole = coverage(fixture, database)
    counts_only = AvailableCoverage(status="available", rules=whole["rules"], families=whole["families"])
    synced = revisions.synced
    cases = [
        reply("repository coverage", database, RULES_CATALOG, [CELLS], synced, whole),
        reply("schema 47 coverage", compatible, RULES_CATALOG, [CELLS], synced, whole),
        reply("crate coverage", database, RULES_CATALOG, [CELLS, "--crate", "crates/alpha"], synced,
              coverage(fixture, database, crate="crates/alpha")),
        reply("counts without the cell projection", database, RULES_CATALOG, [], synced, counts_only),
    ]
    for name in ("lint", "names"):
        tier = fixture.definitions[rule_id(name)]["tier"]
        cases.append(reply(f"tier {tier} rule request", database, RULES_CATALOG, [CELLS, RULE, rule_id(name)], synced,
                           coverage(fixture, database, rule=rule_id(name))))
    cases.extend(sites_cases(fixture, database))
    cases.extend(refused_cases(fixture))
    print(json.dumps(cases, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
