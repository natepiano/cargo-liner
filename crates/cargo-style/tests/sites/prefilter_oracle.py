"""Read the expected pre_filter sites from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import sys
import tomllib
from collections.abc import Callable
from contextlib import closing
from pathlib import Path
from shutil import copyfile
from typing import TypedDict

from oracle import Reply, blob, commit, commit_with, copy_ledger, git, listed, python_refuses, reply, seed_ledger
from oracle import set_synced_commit, write
from scripts.style.git_process import LedgerError
from scripts.style.style_ledger import Ledger


PREFILTER = "global:rust/prefilter.md"
MEMBER = "local:docs/focus/style/prefilter.md"
DEPENDENCIES = "local:docs/focus/style/dependencies.md"
BOTH = "local:docs/style/both.md"
MARKER = "local:docs/style/a-marker.md"
BAD_PATTERN = "local:docs/style/bad-pattern.md"
BAD_GENERATOR = "local:docs/style/bad-generator.md"
BAD_MEMBER_GENERATOR = "local:docs/focus/style/bad-generator.md"
PATTERN = r"(?<=anchor:)\s+\w+\s*$"
MEMBER_MANIFEST = "crates/alpha/Cargo.toml"
MEMBER_SOURCE = "crates/alpha/src/lib.rs"
OTHER_SOURCE = "crates/beta/src/lib.rs"
IGNORED_SOURCE = "crates/alpha/src/ignored.rs"
SUBMODULE = "crates/alpha/src/helpers.rs"
PIN = 'dep = "1" # anchor: café'
CRLF_LINE = "// anchor: ASCII"
CR_LINE = "  // anchor:\u00a0Élan  "
UNICODE_LINE = "\t// anchor:\u2003東京\t"
OTHER_LINE = "// anchor: outside"
FRESH = "fresh prefilter.sqlite"

SiteRow = tuple[str, int, str]


class PrefilterCase(TypedDict):
    name: str
    rule: str
    ledger: str
    expected: Reply


def case(name: str, rule: str, ledger: str, expected: Reply) -> PrefilterCase:
    return {"name": name, "rule": rule, "ledger": ledger, "expected": expected}


def rule_source(tier: str, family: str, fields: str = "") -> str:
    return f"---\ntier: {tier}\nfamily: {family}\n{fields}---\nPrefilter sites fixture\n"


def write_local(repo: Path, rule: str, source: str) -> None:
    write(repo, rule.removeprefix("local:"), source)


def configure(repo: Path) -> None:
    for key, value in (
        ("user.name", "Prefilter Sites Test"),
        ("user.email", "prefilter-sites@example.invalid"),
        ("core.hooksPath", "/dev/null"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)


def write_base(root: Path, repo: Path) -> None:
    configure(repo)
    prefilter = rule_source("B", "names", f"pre_filter: '{PATTERN}'\n")
    write(root / "style", PREFILTER.removeprefix("global:"), prefilter)
    write_local(repo, MEMBER, prefilter)
    write_local(repo, DEPENDENCIES, rule_source("D", "modules",
                "candidates:\n  kind: workspace_deps\n  surface: whole\n"))
    write_local(repo, BOTH, rule_source("B", "names",
                "pre_filter: '('\ncandidates:\n  kind: submodule_names\n  surface: whole\n"))
    write(repo, "Cargo.toml", '[workspace]\nmembers = ["crates/alpha", "crates/beta"]\n')
    for directory, package in (("alpha", "focus"), ("beta", "beta")):
        write(repo, f"crates/{directory}/Cargo.toml", f'[package]\nname = "{package}"\n[dependencies]\n{PIN}\n')
    write(repo, "rustfmt.toml", f'ignore = ["{IGNORED_SOURCE}"]\n')
    write(repo, "notes.md", "anchor: outside_universe\n")


def write_sources(repo: Path) -> None:
    write(repo, MEMBER_SOURCE, f"{CRLF_LINE}\r\n{CR_LINE}\r// no match\u2028{UNICODE_LINE}\n")
    write(repo, OTHER_SOURCE, f"{OTHER_LINE}\n")
    write(repo, IGNORED_SOURCE, "// anchor: ignored\n")
    write(repo, SUBMODULE, "// Include a row from candidates.\n")


def checked_reply(oracle: Ledger, repo: Path, rule: str, expected_rows: tuple[SiteRow, ...]) -> Reply:
    rows = [(row.path, row.line, row.text) for row in listed(oracle, repo, rule)]
    assert rows == list(expected_rows), (rule, rows)
    return reply(oracle, repo, rule)


def saved_reply(root: Path, repo: Path, ledger: Path, rule: str, expected_rows: tuple[SiteRow, ...]) -> Reply:
    reference = root / "read prefilter oracle.sqlite"
    copy_ledger(ledger, reference)
    with Ledger(repo, reference) as oracle:
        return checked_reply(oracle, repo, rule, expected_rows)


def tip_without_manifest_membership(repo: Path, tip: str) -> str:
    manifest = blob(repo, b'[workspace]\nmembers = ["crates/beta"]\n')
    return commit_with(repo, tip, "Cargo.toml", f"100644 blob {manifest}", "Remove alpha from the generator universe")


def legacy_cases(root: Path, repo: Path, tip: str, reference: Path) -> list[PrefilterCase]:
    staging = root / "legacy staging.sqlite"
    schema_51 = root / "legacy schema 51.sqlite"
    rows = ((MEMBER_SOURCE, 3, "saved row A"), (MEMBER_SOURCE, 3, "saved row B"))
    copy_ledger(reference, staging)
    with closing(sqlite3.connect(staging)) as connection:
        mode = connection.execute("PRAGMA journal_mode = WAL").fetchone()
        assert mode == ("wal",), mode
        with connection:
            _ = connection.execute("DELETE FROM refill_candidates")
            _ = connection.execute("DELETE FROM listed_sites WHERE rule = ?", (PREFILTER,))
            _ = connection.executemany(
                "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
                + "SELECT rule, rule_blob, tip, ?, ?, ? FROM listed_rules WHERE rule = ? AND tip = ?",
                [(path, line, text, PREFILTER, tip) for path, line, text in rows],
            )
        expected = saved_reply(root, repo, staging, PREFILTER, rows)
        for suffix in ("", "-wal"):
            _ = copyfile(f"{staging}{suffix}", f"{schema_51}{suffix}")

    schema_47 = root / "legacy schema 47.sqlite"
    seed_ledger(repo, schema_47, tip, 47)
    with closing(sqlite3.connect(schema_47)) as connection, connection:
        _ = connection.execute("ATTACH DATABASE ? AS legacy", (str(staging),))
        _ = connection.execute(
            "INSERT INTO listed_rules (rule, rule_blob, tip) SELECT rule, rule_blob, tip FROM legacy.listed_rules")
        _ = connection.execute(
            "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
            + "SELECT rule, rule_blob, tip, path, line, text FROM legacy.listed_sites")
    return [
        case("schema 51 WAL keeps saved rows with duplicate coordinates", PREFILTER, schema_51.name, expected),
        case("schema 47 keeps saved rows with duplicate coordinates", PREFILTER, schema_47.name,
             saved_reply(root, repo, schema_47, PREFILTER, rows)),
    ]


def unreadable_case(root: Path, repo: Path, tip: str) -> PrefilterCase:
    invalid_utf8 = "crates/alpha/src/unreadable.rs"
    bad_blob = blob(repo, b"// invalid \xff\n")
    broken_tip = commit_with(repo, tip, invalid_utf8, f"100644 blob {bad_blob}", "Add a non-UTF-8 Rust file")
    unreadable = "unreadable prefilter.sqlite"
    seed_ledger(repo, root / unreadable, broken_tip)
    python_refuses(root, unreadable, "repo", PREFILTER, UnicodeDecodeError, "can't decode byte 0xff")
    return case("non-UTF-8 universe blob", PREFILTER, unreadable,
                {"refusal": [f'{PREFILTER}: pre_filter source is not UTF-8: "{invalid_utf8}"']})


def invalid_pattern_cases(root: Path) -> None:
    repo = root / "repo"
    configure(repo)
    write_local(repo, MARKER, rule_source("B", "names", "pre_filter: marker\n"))
    write_local(repo, BAD_PATTERN, rule_source("B", "names", "pre_filter: '('\n"))
    write(repo, "src/lib.rs", "// marker\n")
    bad_pattern_tip = commit(repo, "Add an invalid pre_filter pattern")
    invalid_pattern = "invalid prefilter.sqlite"
    seed_ledger(repo, root / invalid_pattern, bad_pattern_tip)
    reason = f"{BAD_PATTERN}: invalid pre_filter: missing ), unterminated subpattern at position 0"
    cases = []
    for name, rule in (("invalid Python pattern", BAD_PATTERN), ("invalid sibling pre_filter", MARKER)):
        python_refuses(root, invalid_pattern, "repo", rule, LedgerError, reason)
        cases.append(case(name, rule, invalid_pattern, {"refusal": [reason]}))
    print(json.dumps(cases))


def invalid_generator_cases(root: Path) -> None:
    repo = root / "repo"
    configure(repo)
    write(root / "style", PREFILTER.removeprefix("global:"), rule_source("B", "names", f"pre_filter: '{PATTERN}'\n"))
    write(repo, "src/lib.rs", "// anchor: source\n")
    write(repo, "Cargo.toml", '[workspace]\nmembers = ["crates/alpha"]\n')
    write(repo, MEMBER_MANIFEST, '[package]\nname = "focus"\n')
    generator = rule_source("B", "names", "candidates:\n  kind: regex\n  surface: whole\n  pattern: '('\n")
    write_local(repo, BAD_GENERATOR, generator)
    write_local(repo, BAD_MEMBER_GENERATOR, generator)
    bad_generator_tip = commit(repo, "Add failing root and member generators to the catalog")
    invalid_generator = "invalid generator.sqlite"
    seed_ledger(repo, root / invalid_generator, bad_generator_tip)
    cases: list[PrefilterCase] = []
    for rule, rust_error_rule, name in (
        (BAD_GENERATOR, BAD_GENERATOR, f"root generator fails first for {BAD_GENERATOR}"),
        (BAD_MEMBER_GENERATOR, BAD_MEMBER_GENERATOR, "direct member request reports member in Rust and root in Python"),
        (PREFILTER, BAD_GENERATOR, f"root generator fails first for {PREFILTER}"),
    ):
        python_refuses(root, invalid_generator, "repo", rule, LedgerError, f"{BAD_GENERATOR}: candidates regex for")
        fragments = ["candidates regex for", rust_error_rule.removeprefix("local:"), "unclosed group"]
        cases.append(case(name, rule, invalid_generator, {"refusal": fragments}))
    print(json.dumps(cases))


def malformed_manifest_cases(root: Path) -> None:
    repo = root / "repo"
    configure(repo)
    tier_a = "local:docs/style/format.md"
    write_local(repo, MARKER, rule_source("B", "names", "pre_filter: marker\n"))
    write_local(repo, tier_a, "---\ntier: A\nmechanism: rustfmt\n---\n# Format\n")
    write(repo, "src/lib.rs", "// marker\n")
    write(repo, "Cargo.toml", "[workspace\n")
    tip = commit(repo, "Save an invalid root manifest")
    ledger = "invalid root manifest.sqlite"
    seed_ledger(repo, root / ledger, tip)
    write(repo, "Cargo.toml", "[workspace]\n")
    reason = "Expected ']' at the end of a table declaration"
    expected = {"refusal": ['cannot parse committed root manifest "Cargo.toml"']}
    cases = []
    for name, rule in (
        ("malformed committed manifest without generators", MARKER),
        ("malformed committed manifest refuses a tier A rule", tier_a),
    ):
        python_refuses(root, ledger, "repo", rule, tomllib.TOMLDecodeError, reason)
        cases.append(case(name, rule, ledger, expected))
    print(json.dumps(cases))


def listed_sites_cases(root: Path) -> None:
    repo = root / "repo"
    write_base(root, repo)
    empty_tip = tip_without_manifest_membership(repo, commit(repo, "Save a tree with no Rust paths"))
    empty = root / "empty prefilter.sqlite"
    seed_ledger(repo, empty, empty_tip)
    cases = [case("empty universe", PREFILTER, empty.name, saved_reply(root, repo, empty, PREFILTER, ()))]

    write_sources(repo)
    tip = commit(repo, "Save the pre_filter universe and the rule catalog")
    write(repo, MEMBER_SOURCE, "// anchor: dirty\n")
    seed_ledger(repo, root / FRESH, tip)
    reference = root / "prefilter oracle.sqlite"
    copy_ledger(root / FRESH, reference)
    source_rows = ((MEMBER_SOURCE, 1, CRLF_LINE), (MEMBER_SOURCE, 2, CR_LINE), (MEMBER_SOURCE, 4, UNICODE_LINE))
    member_rows = ((MEMBER_MANIFEST, 4, PIN), *source_rows)
    other_row = (OTHER_SOURCE, 1, OTHER_LINE)
    with Ledger(repo, reference) as oracle:
        for name, rule, rows in (
            ("committed Rust and tier D generator paths with Python line and regex semantics", PREFILTER,
             (*member_rows, other_row)),
            ("member reach excludes another member's Rust file", MEMBER, member_rows),
            ("candidates take precedence over an invalid pre_filter", BOTH,
             ((SUBMODULE, 1, "anti-pattern module name `helpers.rs`"),)),
        ):
            cases.append(case(name, rule, FRESH, checked_reply(oracle, repo, rule, rows)))
    cases.extend(legacy_cases(root, repo, tip, reference))

    next_tip = tip_without_manifest_membership(repo, tip)
    next_ledger = "changed membership.sqlite"
    seed_ledger(repo, root / next_ledger, next_tip)
    set_synced_commit(reference, next_tip)
    with Ledger(repo, reference) as oracle:
        cases.append(case("membership changes with the workspace manifest", PREFILTER, next_ledger,
                          checked_reply(oracle, repo, PREFILTER, (*source_rows, other_row))))
    cases.append(unreadable_case(root, repo, tip))
    print(json.dumps(cases))


def run(fixture: Callable[[Path], None]) -> None:
    fixture(Path(sys.argv[1]))
