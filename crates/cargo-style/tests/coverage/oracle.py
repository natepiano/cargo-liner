"""Build private coverage fixtures and read the expected cells and counts."""

from contextlib import closing
from dataclasses import dataclass, replace
from hashlib import sha256
import json
from pathlib import Path
from shutil import copyfile, copytree
import sqlite3
import subprocess
import sys
from tomllib import TOMLDecodeError
from typing import Literal, NotRequired, TypedDict

from scripts.style import rules
from scripts.style.git_process import GitReadFailed, LedgerError
from scripts.style.style_config import ReaderSource
from scripts.style.style_ledger import (
    CellInputScope,
    CoverageScope,
    CrateCoverage,
    Ledger,
    ListedSite,
    MIGRATIONS,
    ReadFiles,
    WholeRepository,
    base_rule,
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
CELLS = "--cells"
RULE = "--rule"
ALPHA_MANIFEST = "crates/alpha/Cargo.toml"
EMPTY_CRATE = "crates/no-source"
NESTED_CRATE = "crates/alpha/nested"
IGNORED = ALPHA + "ignored.rs"
REMOVED_SITE = ALPHA + "removed-site.rs"
MATCHING_SOURCE = ALPHA + "lib.rs"
NOTES = "notes.md"
UNCLASSIFIED = "global:rust/guide.md"
UNCLASSIFIED_GENERATOR = "global:rust/affixes.md"
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


class Observed(TypedDict):
    synced_tip: str
    ledger_revision: Literal["absent"]


class ReplyBody(TypedDict):
    schema: int
    observed: Observed
    coverage: AvailableCoverage


class ReplyResult(TypedDict):
    kind: Literal["reply"]
    body: ReplyBody


class SitesResult(TypedDict):
    kind: Literal["reply"]
    body: dict[str, object]


class RefusedResult(TypedDict):
    kind: Literal["refused"]
    diagnostic: str


class CoverageCase(TypedDict):
    name: str
    ledger: str
    rules: str
    flags: list[str]
    result: ReplyResult | RefusedResult | SitesResult
    command: NotRequired[Literal["sites"]]


@dataclass(frozen=True)
class Fixture:
    root: Path
    repo: Path
    database: Path
    definitions: Catalog
    revisions: Revisions
    checkout: Path


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
        "docs/style/sites-b.md": regex_rule("B", "types", "file", "whole", "coverage_marker", "*.rs"),
        "docs/style/sites-d.md": regex_rule("D", "modules", "file", "whole", "coverage_marker", "notes.md"),
        "docs/style/partial.md": regex_rule("B", "types", "file", "partial", "coverage_marker", "*.rs")
        + "\n  uncovered: Review remaining Rust files.",
        "docs/alpha/style/prefilter.md": "tier: B\nfamily: names\npre_filter: coverage_marker",
        "docs/alpha/style/member-sites.md": regex_rule("D", "modules", "file", "whole", "member_only", "*.md"),
        "docs/style/no-generator.md": "tier: D\nfamily: modules\ninput_scope: crate",
        "docs/style/manifests-d.md": "tier: D\nfamily: modules\ninput_scope: manifest\n"
        + "candidates:\n  kind: workspace_deps\n  surface: whole",
    }
    for path, metadata in definitions.items():
        write(repo, path, f"---\n{metadata}\n---\n\nUse the fixture rule in {path}.\n")


def regex_rule(tier: str, family: str, scope: str, surface: str, pattern: str, glob: str) -> str:
    return (f"tier: {tier}\nfamily: {family}\ninput_scope: {scope}\ncandidates:\n"
            f"  kind: regex\n  surface: {surface}\n  pattern: '{pattern}'\n  globs: ['{glob}']")


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
    workspace = ('[package]\nname = "coverage-fixture"\n[dependencies]\nroot_dep = "1"\n'
                 f'[workspace]\nmembers = ["crates/alpha", "crates/beta", "{NESTED_CRATE}", "{EMPTY_CRATE}"]\n')
    write(repo, "Cargo.toml", workspace)
    for directory in ("crates/alpha", "crates/beta", NESTED_CRATE, EMPTY_CRATE):
        write(repo, f"{directory}/Cargo.toml", f'[package]\nname = "{Path(directory).name}"\nversion = "0.1.0"\n'
              + '[dependencies]\ncoverage_dep = "1" # coverage_marker\n')
    install_rules(repo)
    for file in ("lib", "stale", "branch", "removed", "branch-removed", "inactive", "missing", "explicit", "comment", "inner"):
        write(repo, f"{ALPHA}{file}.rs", CODE)
    write(repo, f"{BETA}lib.rs", CODE)
    write_membership_sources(repo)
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
    write(repo, "rustfmt.toml", f'max_width = 95\nignore = ["{ALPHA}ignored*", "crates/alpha/ignored.md"]\n')
    write(repo, "Cargo.toml", workspace + '\n[workspace.lints.rust]\nunsafe_code = "deny"\n')
    write(repo, REMOVED_SITE, CODE)
    write(repo, ALPHA + "comments/lib.rs", "// A regular comment in a separate module.\n" + CODE)
    write(repo, NOTES, "coverage_marker changed\n")
    beta_manifest = f"{BETA.removesuffix('src/')}Cargo.toml"
    write(repo, beta_manifest, (repo / beta_manifest).read_text(encoding="utf-8").replace('"1"', '"2"'))
    names = "docs/style/names.md"
    write(repo, names, (repo / names).read_text(encoding="utf-8").replace("tier: C", "tier: C\ntags: [test]"))
    partial = "docs/style/partial.md"
    write(repo, partial, (repo / partial).read_text(encoding="utf-8").replace("tier: B", "tier: B\ntags: [test]"))
    synced = commit(repo, "Advance the synced tip")
    _ = git(repo, "switch", "-qc", "style/main")
    write(repo, f"{ALPHA}branch.rs", CODE.replace("work", "on_branch"))
    (repo / f"{ALPHA}branch-removed.rs").unlink()
    branch = commit(repo, "Judge changes on the style branch")
    _ = git(repo, "switch", "-q", "main")
    return repo, Revisions(base, removed, synced, branch)


def write_membership_sources(repo: Path) -> None:
    write(repo, MATCHING_SOURCE, CODE + "// coverage_marker first\n// coverage_marker second\n")
    write(repo, REMOVED_SITE, CODE + "// coverage_marker removed at the synced tip\n")
    write(repo, IGNORED, "// coverage_marker ignored\n")
    write(repo, ALPHA + "comments/lib.rs", CODE)
    write(repo, f"{NESTED_CRATE}/src/lib.rs", CODE)
    write(repo, "vendor/sample/lib.rs", CODE)
    write(repo, NOTES, "coverage_marker original\n")
    for path in ("crates/alpha/ignored.md", "crates/beta/outside.md", "crates/alpha/member.md"):
        write(repo, path, "member_only coverage_marker\n")


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
        Judgment("partial", MATCHING_SOURCE, base, verdict="held"),
        Judgment("partial#uncovered", MATCHING_SOURCE, base, reads=("context.md",)),
        Judgment("sites-d", MATCHING_SOURCE, base, scope=CellInputScope.MODULE),
        Judgment("sites-d", ALPHA + "branch.rs", branch, scope=CellInputScope.MODULE),
        Judgment("sites-d", ALPHA + "explicit.rs", base, declared="file"),
        Judgment("sites-d", ALPHA + "comments/lib.rs", base, scope=CellInputScope.MODULE),
        Judgment("sites-d", NOTES, base),
        Judgment("sites-d", "crates/alpha", revisions.synced, scope=CellInputScope.CRATE),
        Judgment("alpha/prefilter", ALPHA_MANIFEST, base),
        Judgment("no-generator", MATCHING_SOURCE, base, scope=CellInputScope.MODULE),
        Judgment("manifests-d", "crates/alpha", base, scope=CellInputScope.MANIFEST),
        Judgment("manifests-d", EMPTY_CRATE, revisions.synced, scope=CellInputScope.MANIFEST),
    )


def rule_id(name: str) -> str:
    base, separator, component = name.partition("#")
    member, slash, stem = base.rpartition("/")
    return f"local:docs/{member}{slash}style/{stem}.md{separator}{component}"


def pin_at(repo: Path, commit_id: str, name: str) -> str:
    path = base_rule(rule_id(name)).removeprefix("local:")
    return git(repo, "rev-parse", f"{commit_id}:{path}")


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
            definition = rule_definition(rule, mechanisms.get(rule.name))
            if rule.origin == "local":
                old = git(repo, "rev-parse", f"{revisions.base}:{rule.path.relative_to(repo).as_posix()}")
                definition["identities"][old] = history.identity(old, rule) or None
            result[rule.rule_id] = definition
        return result
    finally:
        history.close()


def rule_definition(rule: rules.Rule, mechanism: str | None) -> RuleDefinition:
    reach = rule.reach.crate_dir.as_posix() if isinstance(rule.reach, rules.MemberReach) else None
    return RuleDefinition(tier=rule.tier, family=rule.family, blob=rule.blob, input_scope=rule.input_scope, reach=reach,
                          surface=rule.candidates.surface if rule.candidates else None, mechanism=mechanism,
                          identities={rule.blob: rule.judgment_identity or None})


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
    reference = fixture.root / "coverage-oracle.sqlite"
    copy_ledger(database, reference, "SELECT 1")
    base = base_rule(rule) if rule else None
    with Ledger(fixture.repo, reference) as ledger, ledger.rules_from_checkout(fixture.checkout):
        cells = sorted((CellRow(rule=cell.rule, path=cell.path, crate=cell.crate, tier=cell.tier, family=cell.family,
                                state=cell.state) for cell in ledger.cells(rule=rule, crate=crate)),
                       key=lambda row: (row["rule"], row["path"]))
        counts = sorted((RuleCountRow(rule_id=row.rule_id, tier=row.tier, input_scope=row.input_scope,
                                      current=row.current, stale=row.stale, open=row.open,
                                      awaiting_merge=row.awaiting_merge, held=row.held)
                         for row in ledger.rule_counts(scope) if base is None or row.rule_id == base),
                        key=lambda row: row["rule_id"])
    return AvailableCoverage(status="available", cells=cells, rules=counts,
                             families=family_rows(counts, fixture.definitions))


def reply(name: str, ledger: Path, catalog_name: str, flags: list[str], tip: str,
          answer: AvailableCoverage) -> CoverageCase:
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
    synced = fixture.revisions.synced
    cases = []
    for label, name, crate in (
        ("whole tier B deduplicates multiple sites in one file", "sites-b", None),
        ("whole tier B has zero cells outside its membership", "sites-b", "crates/beta"),
        ("tier D joins Rust and Markdown paths and compares their scopes", "sites-d", None),
        ("tier D without candidates covers Rust paths", "no-generator", None),
        ("manifest tier D includes the root and a crate without Rust files", "manifests-d", None),
        ("member prefilter uses the non-Rust paths of sibling generators", "alpha/prefilter", None),
        ("member generator excludes ignored and outside paths", "alpha/member-sites", None),
        ("partial B projects both components and folds counts", "partial", None),
        ("explicit uncovered projects only C cells with both base counts", "partial#uncovered", "crates/alpha"),
    ):
        rule = rule_id(name)
        flags = [CELLS, RULE, rule, *(('--crate', crate) if crate else ())]
        cases.append(reply(label, latest, RULES_CATALOG, flags, synced, coverage(fixture, latest, crate, rule)))

    previous = fixture.root / "before-site-removal.sqlite"
    copy_ledger(latest, previous, "UPDATE sync SET tip=?", (fixture.revisions.base,))
    whole_b = rule_id("sites-b")
    old = coverage(fixture, previous, rule=whole_b)
    assert REMOVED_SITE in {row["path"] for row in old["cells"]}, old
    assert [row["path"] for row in coverage(fixture, latest, rule=whole_b)["cells"]] == [MATCHING_SOURCE]
    cases.append(reply("whole tier B includes a site at the earlier tip", previous, RULES_CATALOG, [CELLS, RULE, whole_b],
                       fixture.revisions.base, old))
    cases.append(reply("all five tiers with a nested crate filter", latest, RULES_CATALOG,
                       [CELLS, "--crate", NESTED_CRATE], synced, coverage(fixture, latest, crate=NESTED_CRATE)))
    cases.append(reply("known unclassified rule has no cells or count row", latest, RULES_CATALOG,
                       [CELLS, RULE, UNCLASSIFIED], synced, coverage(fixture, latest, rule=UNCLASSIFIED)))
    unclassified = coverage(fixture, latest, rule=UNCLASSIFIED_GENERATOR)
    assert unclassified == AvailableCoverage(status="available", cells=[], rules=[], families=[]), unclassified
    cases.append(reply("unclassified field_affixes has empty coverage", latest, RULES_CATALOG,
                       [CELLS, RULE, UNCLASSIFIED_GENERATOR], synced, unclassified))
    partial = rule_id("partial")
    counts = coverage(fixture, latest, rule=partial)
    del counts["cells"]
    cases.append(reply("partial B counts without the cell projection", latest, RULES_CATALOG, [RULE, partial], synced, counts))
    return cases


def insert_legacy_markers(fixture: Fixture, connection: sqlite3.Connection) -> None:
    tip = fixture.revisions.synced
    _ = connection.executemany("INSERT INTO listed_rules (rule,rule_blob,tip) VALUES (?,?,?)", (
        (rule.rule_id, rule.blob, tip) for rule in rules.load_rules(fixture.checkout)
        if (rule.tier in ("B", "D") and rule.candidates is not None) or (rule.tier == "B" and rule.pre_filter is not None)
    ))


def legacy_cases(fixture: Fixture) -> list[CoverageCase]:
    root, synced = fixture.root, fixture.revisions.synced
    rule = rule_id("sites-b")
    flags = [CELLS, RULE, rule]
    empty = root / "empty markers.sqlite"
    copy_ledger(fixture.database, empty, "SELECT 1")
    with closing(sqlite3.connect(empty)) as connection, connection:
        insert_legacy_markers(fixture, connection)
    empty_answer = coverage(fixture, empty, rule=rule)
    assert not empty_answer["cells"], empty_answer

    staging = root / "legacy staging.sqlite"
    populated = root / "legacy WAL.sqlite"
    tier_d = rule_id("sites-d")
    copy_ledger(empty, staging, "SELECT 1")
    with closing(sqlite3.connect(staging)) as connection:
        mode = connection.execute("PRAGMA journal_mode=WAL").fetchone()
        assert mode == ("wal",), mode
        with connection:
            _ = connection.executemany("INSERT INTO listed_sites (rule,rule_blob,tip,path,line,text) VALUES (?,?,?,?,?,?)",
                                       [(name, fixture.definitions[name]["blob"], synced, path, 1, "saved legacy row")
                                        for name, path in ((rule, IGNORED), (tier_d, "crates/alpha"))])
        saved_answer = coverage(fixture, staging, rule=rule)
        crate_answer = coverage(fixture, staging, rule=tier_d)
        assert [row["path"] for row in saved_answer["cells"]] == [IGNORED], saved_answer
        assert [(row["crate"], row["state"]) for row in crate_answer["cells"] if row["path"] == "crates/alpha"] == [
            ("crates/alpha", "current"),
        ], crate_answer
        for suffix in ("", "-wal"):
            _ = copyfile(f"{staging}{suffix}", f"{populated}{suffix}")

    stale = root / "stale legacy pin.sqlite"
    stale_pin = "0" * 40
    copy_ledger(staging, stale, "UPDATE listed_rules SET rule_blob=? WHERE rule=?", (stale_pin, rule))
    with closing(sqlite3.connect(stale)) as connection, connection:
        _ = connection.execute("UPDATE listed_sites SET rule_blob=? WHERE rule=?", (stale_pin, rule))
    indexed = root / "legacy with candidate cache.sqlite"
    catalog_id = json.dumps((str(fixture.checkout.resolve()), str((root / "global-rules").resolve())))
    copy_ledger(staging, indexed, "INSERT INTO refill_candidates VALUES (?,?,?,'stale',0,'[]')",
                (catalog_id, rule, "fixture partition"))
    cases = [
        reply("complete empty legacy markers yield zero B cells", empty, RULES_CATALOG, flags, synced, empty_answer),
        reply("legacy WAL supplies an ignored Rust path", populated, RULES_CATALOG, flags, synced, saved_answer),
        reply("legacy D path that equals its crate uses crate scope", populated, RULES_CATALOG, [CELLS, RULE, tier_d],
              synced, crate_answer),
    ]
    for label, database in (("stale marker pin refreshes membership", stale), ("candidate cache forces refresh", indexed)):
        answer = coverage(fixture, database, rule=rule)
        assert [row["path"] for row in answer["cells"]] == [MATCHING_SOURCE], answer
        cases.append(reply(label, database, RULES_CATALOG, flags, synced, answer))
    return cases


def copy_rule_checkout(fixture: Fixture, name: str) -> Path:
    checkout = fixture.root / name
    _ = copytree(fixture.repo / "docs", checkout / "docs")
    for manifest in fixture.repo.rglob("Cargo.toml"):
        write(checkout, manifest.relative_to(fixture.repo).as_posix(), manifest.read_text(encoding="utf-8"))
    return checkout


def checkout_with_rule(fixture: Fixture, name: str, rule: str, source: str) -> Fixture:
    checkout = copy_rule_checkout(fixture, name)
    write(checkout, rule.removeprefix("local:"), f"---\n{source}\n---\n\nUse the alternate checkout rule.\n")
    definition = next(item for item in rules.load_rules(checkout) if item.rule_id == rule)
    definitions = {**fixture.definitions, rule: rule_definition(definition, None)}
    write_catalog(fixture.root, f"{name}.json", definitions)
    return replace(fixture, definitions=definitions, checkout=checkout)


def alternate_checkout_case(fixture: Fixture) -> CoverageCase:
    rule = rule_id("alpha/prefilter")
    alternate = checkout_with_rule(fixture, "alternate rules", rule, "tier: B\nfamily: names\npre_filter: member_only")
    return reply("the separate checkout controls membership and pin", fixture.database, "alternate rules.json",
                 [CELLS, RULE, rule, "--checkout", str(alternate.checkout)], fixture.revisions.synced,
                 coverage(alternate, fixture.database, rule=rule))


def symlinked_rule_cases(fixture: Fixture) -> list[CoverageCase]:
    checkout = copy_rule_checkout(fixture, "symlink rules")
    (checkout / "docs/style/symlinked-b.md").symlink_to("sites-b.md")
    rule = rule_id("sites-b")
    alternate = replace(fixture, checkout=checkout)
    flags = [CELLS, RULE, rule, "--checkout", str(checkout)]
    diagnostic = "UNIQUE constraint failed: listed_rules.rule, listed_rules.rule_blob, listed_rules.tip"
    python_refuses_coverage(alternate, fixture.database, rule, sqlite3.IntegrityError, diagnostic)
    synced = fixture.revisions.synced
    database = fixture.root / "symlink rules.sqlite"
    copy_ledger(fixture.database, database, "SELECT 1")
    with closing(sqlite3.connect(database)) as connection, connection:
        insert_legacy_markers(fixture, connection)
        _ = connection.execute("INSERT INTO listed_sites (rule,rule_blob,tip,path,line,text) VALUES (?,?,?,?,?,?)",
                               (rule, fixture.definitions[rule]["blob"], synced, MATCHING_SOURCE, 2,
                                "coverage_marker first"))
    answer = coverage(alternate, database, rule=rule)
    assert [row["path"] for row in answer["cells"]] == [MATCHING_SOURCE, MATCHING_SOURCE], answer
    assert [row["open"] for row in answer["rules"]] == [2, 2], answer
    return [
        refused("fresh symlinked B rule repeats the listed rule key", fixture.database, flags, diagnostic),
        reply("symlinked B rule repeats the cell and count row", database, RULES_CATALOG, flags, synced, answer),
    ]


def cached_catalog_without_generators_case(fixture: Fixture) -> CoverageCase:
    repo = fixture.repo
    _ = git(repo, "switch", "-q", "--detach", fixture.revisions.synced)
    unrelated = "unrelated.txt"
    write(repo, unrelated, "Only this commit refers to this blob.\n")
    tip = commit(repo, "Record an unrelated coverage blob")
    blob = git(repo, "rev-parse", f"{tip}:{unrelated}")
    _ = git(repo, "switch", "-q", "main")
    (repo / ".git/objects" / blob[:2] / blob[2:]).unlink()

    checkout = fixture.root / "rules without generators"
    rule = rule_id("names")
    rule_path = rule.removeprefix("local:")
    write(checkout, rule_path, (repo / rule_path).read_text(encoding="utf-8"))
    alternate = replace(fixture, checkout=checkout)
    database = fixture.root / "candidate cache without generators.sqlite"
    copy_ledger(fixture.database, database, "UPDATE sync SET tip=?", (tip,))
    catalog_id = json.dumps((str(checkout.resolve()), str((fixture.root / "global-rules").resolve())))
    with closing(sqlite3.connect(database)) as connection, connection:
        _ = connection.execute("DELETE FROM judgments")
        assert connection.execute("SELECT COUNT(*) FROM listed_rules").fetchone() == (0,)
        _ = connection.execute("INSERT INTO refill_candidates VALUES (?,?,?,'stale',0,'[]')",
                               (catalog_id, rule_id("sites-b"), "fixture partition"))
    answer = coverage(alternate, database, rule=rule)
    assert {row["state"] for row in answer["cells"]} == {"open"}, answer
    assert len(answer["rules"]) == 1 and answer["rules"][0]["open"] == len(answer["cells"]), answer
    return reply("old candidate cache without generators or markers does not read the missing blob",
                 database, RULES_CATALOG, [CELLS, RULE, rule, "--checkout", str(checkout)], tip, answer)


def python_refuses_coverage(fixture: Fixture, database: Path, rule: str | None,
                            exception: type[Exception], diagnostic: str) -> None:
    try:
        _ = coverage(fixture, database, rule=rule)
    except exception as error:
        assert diagnostic in str(error), str(error)
    else:
        raise AssertionError(f"Python accepted coverage for {rule} with {fixture.checkout}")


def python_sites(fixture: Fixture, database: Path, rule: str) -> tuple[ListedSite, ...]:
    reference = fixture.root / "direct-sites-oracle.sqlite"
    copy_ledger(database, reference, "SELECT 1")
    with Ledger(fixture.repo, reference) as ledger, ledger.rules_from_checkout(fixture.checkout):
        return ledger.listed_sites(rule)


def listed_sites_value(repo: Path, rule: str, pin: str, tip: str, rows: tuple[ListedSite, ...]) -> dict[str, object]:
    sites: list[dict[str, object]] = []
    for row in rows:
        blob = git(repo, "rev-parse", f"{tip}:{row.path}")
        identity = json.dumps([rule, pin, row.path, blob, row.line, row.text], ensure_ascii=False, separators=(",", ":"))
        sites.append({"file": row.path, "line": row.line, "site_id": sha256(identity.encode()).hexdigest()})
    return {"status": "listed", "sites": sites}


def direct_sites_reply(name: str, database: Path, checkout: Path, rule: str, pin: str, tip: str,
                       outcome: dict[str, object]) -> CoverageCase:
    body = {"schema": 1, "rule": rule, "observed": {"synced_tip": tip, "rule_pin": pin,
                                                   "ledger_revision": "absent"}, **outcome}
    return CoverageCase(name=name, ledger=database.name, rules=RULES_CATALOG, command="sites",
                        flags=["--checkout", str(checkout), rule], result=SitesResult(kind="reply", body=body))


def direct_sites_cases(fixture: Fixture, alternate: Fixture, unsupported: str) -> list[CoverageCase]:
    selected = rule_id("partial")
    synced = fixture.revisions.synced
    pin = fixture.definitions[selected]["blob"]
    rows = python_sites(fixture, fixture.database, selected)
    results: list[tuple[str, str, str, dict[str, object]]] = [
        ("direct sites lists the requested generator despite an unsupported sibling", selected, pin,
         listed_sites_value(fixture.repo, selected, pin, synced, rows)),
        ("direct sites keeps a typed unsupported generator", unsupported, alternate.definitions[unsupported]["blob"],
         {"status": "unavailable", "reason": "unsupported_generator"}),
    ]
    return [direct_sites_reply(name, fixture.database, alternate.checkout, rule, rule_pin, synced, outcome)
            for name, rule, rule_pin, outcome in results]


def refresh_refusals(fixture: Fixture) -> list[CoverageCase]:
    rule = rule_id("sites-b")
    prefilter = rule_id("alpha/prefilter")
    unsupported = regex_rule("B", "types", "file", "whole", "coverage_marker", "*.rs")
    cases: list[CoverageCase] = []
    for name, changed, source, python_error, rust_error in (
        ("unsupported generator", rule, unsupported.replace("kind: regex", "kind: unknown_fixture"),
         "has no generator in candidate_generators.py", "has no generator in candidate_generators.py"),
        ("invalid regex generator", rule, regex_rule("B", "types", "file", "whole", "(", "*.rs"),
         "unclosed group", "unclosed group"),
        ("invalid sibling prefilter", prefilter, "tier: B\nfamily: names\npre_filter: '(' ",
         "invalid pre_filter: missing ), unterminated subpattern", "invalid pre_filter: missing ), unterminated subpattern"),
    ):
        alternate = checkout_with_rule(fixture, name, changed, source)
        for selected in (rule_id("names"), None):
            python_refuses_coverage(alternate, fixture.database, selected, LedgerError, python_error)
            flags = [CELLS, "--checkout", str(alternate.checkout), *((RULE, selected) if selected else ())]
            row = refused(f"{name} refuses {selected or 'whole coverage'}", fixture.database, flags, rust_error)
            row["rules"] = f"{name}.json"
            cases.append(row)
        if name == "unsupported generator":
            cases.extend(direct_sites_cases(fixture, alternate, rule))
    return cases


def input_refusals(fixture: Fixture) -> list[CoverageCase]:
    cases = []
    for name in ("sites-b#uncovered", "alpha/prefilter#uncovered", "sites-d#uncovered", "absent#uncovered"):
        rule = rule_id(name)
        rejected = base_rule(rule) if name.startswith("absent") else rule
        diagnostic = f"unknown rule {rejected}"
        python_refuses_coverage(fixture, fixture.database, rule, LedgerError, diagnostic)
        cases.append(refused(f"invalid component {name}", fixture.database, [CELLS, RULE, rule], "unknown rule"))

    source = ALPHA + "unreadable.rs"
    _ = (fixture.repo / source).write_bytes(b"// unreadable \xff\n")
    tip = commit(fixture.repo, "Commit non-UTF-8 source in the prefilter universe")
    unreadable = fixture.root / "unreadable source.sqlite"
    copy_ledger(fixture.database, unreadable, "UPDATE sync SET tip=?", (tip,))
    for selected in (rule_id("names"), None):
        python_refuses_coverage(fixture, unreadable, selected, UnicodeDecodeError, "can't decode byte 0xff")
        flags = [CELLS, *((RULE, selected) if selected else ())]
        cases.append(refused(f"unreadable source refuses {selected or 'whole coverage'}", unreadable, flags,
                             f'pre_filter source is not UTF-8: "{source}"'))
    return cases


def toml_refusals(fixture: Fixture) -> list[CoverageCase]:
    repo, synced = fixture.repo, fixture.revisions.synced
    ledgers = []
    for name, path in (("root manifest", "Cargo.toml"), ("member manifest", ALPHA_MANIFEST),
                       ("ignore file", "rustfmt.toml")):
        _ = git(repo, "switch", "-q", "--detach", synced)
        contents = (repo / path).read_text(encoding="utf-8")
        write(repo, path, contents + '\n[fixture]\ninline = { value = "text", }\n')
        invalid_tip = commit(repo, f"Save TOML 1.1 in {path}")
        database = fixture.root / f"TOML 1.1 {name}.sqlite"
        copy_ledger(fixture.database, database, "UPDATE sync SET tip=?", (invalid_tip,))
        ledgers.append((name, path, database))
    _ = git(repo, "switch", "-q", "main")
    rule = rule_id("names")
    cases = []
    for name, path, database in ledgers:
        python_refuses_coverage(fixture, database, rule, TOMLDecodeError, "Invalid initial character for a key part")
        cases.append(refused(f"TOML 1.1 {name} fails before selected coverage", database, [CELLS, RULE, rule], path))
    checkout = copy_rule_checkout(fixture, "TOML 1.1 checkout")
    contents = (checkout / ALPHA_MANIFEST).read_text(encoding="utf-8")
    write(checkout, ALPHA_MANIFEST, contents + '\n[fixture]\ninline = { value = "text", }\n')
    member = rule_id("alpha/prefilter")
    python_refuses_coverage(replace(fixture, checkout=checkout), fixture.database, member, TOMLDecodeError,
                            "Invalid initial character for a key part")
    cases.append(refused("TOML 1.1 member manifest in the rule checkout fails with a valid synced manifest",
                         fixture.database, [CELLS, RULE, member, "--checkout", str(checkout)], ALPHA_MANIFEST))
    return cases


def non_utf8_ignore_cases(fixture: Fixture) -> list[CoverageCase]:
    repo = fixture.repo
    _ = git(repo, "switch", "-q", "--detach", fixture.revisions.synced)
    _ = (repo / "rustfmt.toml").write_bytes(b"ignore = []\n# Invalid UTF-8: \xff\n")
    tip = commit(repo, "Save a comment with invalid UTF-8 in the ignore file")
    _ = git(repo, "switch", "-q", "main")
    database = fixture.root / "non-UTF-8 ignore file.sqlite"
    copy_ledger(fixture.database, database, "UPDATE sync SET tip=?", (tip,))
    rule = rule_id("sites-b")
    python_refuses_coverage(fixture, database, rule, UnicodeDecodeError, "can't decode byte 0xff")
    try:
        _ = python_sites(fixture, database, rule)
    except UnicodeDecodeError as error:
        assert error.object[error.start] == 0xff, str(error)
    else:
        raise AssertionError("Python listed sites with an invalid UTF-8 ignore comment")
    sites = refused("sites refuses a non-UTF-8 comment in rustfmt.toml", database, [rule], "rustfmt.toml")
    sites["command"] = "sites"
    return [refused("coverage refuses a non-UTF-8 comment in rustfmt.toml", database,
                    [CELLS, RULE, rule], "rustfmt.toml"), sites]


def reversed_range_case(fixture: Fixture) -> CoverageCase:
    repo = fixture.repo
    _ = git(repo, "switch", "-q", "--detach", fixture.revisions.synced)
    excluded = ALPHA + "-glob.rs"
    kept = ALPHA + "b-glob.rs"
    for path in (excluded, kept):
        write(repo, path, CODE)
    write(repo, "rustfmt.toml", f'ignore = ["{ALPHA}[z-a-c]glob.rs"]\n')
    tip = commit(repo, "Save the ignore class with a reversed first range")
    _ = git(repo, "switch", "-q", "main")
    database = fixture.root / "reversed range.sqlite"
    copy_ledger(fixture.database, database, "UPDATE sync SET tip=?", (tip,))
    rule = rule_id("names")
    answer = coverage(fixture, database, rule=rule)
    paths = {row["path"] for row in answer["cells"]}
    assert kept in paths and excluded not in paths, paths
    return reply("coverage ignore class matches a literal hyphen after a removed range", database,
                 RULES_CATALOG, [CELLS, RULE, rule], tip, answer)


def replacement_blob_cases(fixture: Fixture) -> list[CoverageCase]:
    repo = fixture.repo
    _ = git(repo, "switch", "-q", "--detach", fixture.revisions.synced)
    replaced = ALPHA + "replaced-affix.rs"
    kept = ALPHA + "kept-affix.rs"
    original = "struct ReplacedAffix {\n    shared_left: u8,\n    shared_right: u8,\n}\n"
    write(repo, replaced, original)
    write(repo, kept, original.replace("ReplacedAffix", "KeptAffix"))
    tip = commit(repo, "Save two sites with shared field prefixes")
    blob = git(repo, "rev-parse", f"{tip}:{replaced}")
    write(repo, replaced, original.replace("shared_", ""))
    replacement = git(repo, "hash-object", "-w", replaced)
    _ = git(repo, "switch", "-q", "--discard-changes", "main")

    rule = rule_id("replace-affixes")
    alternate = checkout_with_rule(fixture, "replacement rules", rule,
                                   "tier: B\nfamily: types\ninput_scope: file\ncandidates:\n"
                                   + "  kind: field_affixes\n  surface: whole")
    database = fixture.root / "replace blob.sqlite"
    copy_ledger(fixture.database, database, "UPDATE sync SET tip=?", (tip,))
    rows = python_sites(alternate, database, rule)
    assert [(row.path, row.line) for row in rows] == [(kept, 1), (replaced, 1)], rows
    _ = git(repo, "replace", blob, replacement)
    rows = python_sites(alternate, database, rule)
    assert [(row.path, row.line) for row in rows] == [(kept, 1)], rows
    answer = coverage(alternate, database, rule=rule)
    assert [row["path"] for row in answer["cells"]] == [kept], answer
    return [
        direct_sites_reply("sites follows a replace ref and removes the changed affix site", database,
                           alternate.checkout, rule, alternate.definitions[rule]["blob"], tip,
                           listed_sites_value(repo, rule, alternate.definitions[rule]["blob"], tip, rows)),
        reply("coverage follows a replace ref and removes the changed affix cell", database,
              "replacement rules.json", [CELLS, RULE, rule, "--checkout", str(alternate.checkout)], tip, answer),
    ]


def missing_sync_case(fixture: Fixture) -> CoverageCase:
    database = fixture.root / "missing sync.sqlite"
    copy_ledger(fixture.database, database, "DELETE FROM sync")
    rule = rule_id("names")
    python_refuses_coverage(fixture, database, rule, LedgerError, "no synced tip")
    return refused("coverage reports no synced tip when the sync row is missing", database,
                   [CELLS, RULE, rule], "no synced tip")


def judgment_cases(fixture: Fixture) -> list[CoverageCase]:
    database = fixture.database
    compatible = fixture.root / f"schema-{COMPATIBLE_SCHEMA}.sqlite"
    build_ledger(fixture.repo, compatible, fixture.revisions, COMPATIBLE_SCHEMA)
    whole = coverage(fixture, database)
    counts_only = AvailableCoverage(status="available", rules=whole["rules"], families=whole["families"])
    synced = fixture.revisions.synced
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
    cases.extend(legacy_cases(fixture))
    cases.extend(refused_cases(fixture))
    return cases


def membership_cases(fixture: Fixture) -> list[CoverageCase]:
    return [
        *sites_cases(fixture, fixture.database),
        alternate_checkout_case(fixture),
        *symlinked_rule_cases(fixture),
        cached_catalog_without_generators_case(fixture),
    ]


def refresh_cases(fixture: Fixture) -> list[CoverageCase]:
    return [
        *refresh_refusals(fixture),
        *toml_refusals(fixture),
        *non_utf8_ignore_cases(fixture),
        reversed_range_case(fixture),
        *replacement_blob_cases(fixture),
        missing_sync_case(fixture),
        *input_refusals(fixture),
    ]


def main(root: Path, group: str) -> None:
    read_cases = {"judgments": judgment_cases, "membership": membership_cases, "refresh": refresh_cases}[group]
    repo, revisions = repository(root)
    write(root / "global-rules", "rust/guide.md", "# Unclassified fixture rule\n")
    write(root / "global-rules", "rust/affixes.md", "---\ncandidates:\n  kind: field_affixes\n---\n\n# Field affixes\n")
    latest = max(MIGRATIONS)
    database = root / f"schema-{latest}.sqlite"
    build_ledger(repo, database, revisions, latest)
    fixture = Fixture(root, repo, database, catalog(repo, revisions), revisions, repo)
    write_catalog(root, RULES_CATALOG, fixture.definitions)
    cases = read_cases(fixture)
    print(json.dumps(cases, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2])
