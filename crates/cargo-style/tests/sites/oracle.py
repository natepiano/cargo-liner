"""Build the sites fixture and read its expected replies from Python."""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import subprocess
import sys
from contextlib import closing
from pathlib import Path

from scripts.style import candidate_generators, rules
from scripts.style.git_process import GitReadFailed, LedgerError
from scripts.style.style_ledger import Ledger, ListedSite, MIGRATIONS


REGEX = "global:rust/marker.md"
LOCAL = "local:docs/focus/style/marker.md"
LOOKBEHIND = "local:docs/style/lookbehind.md"
STAMP = "2026-01-01T00:00:00+00:00"
MARKER_RULE = r"""---
tier: B
family: names
candidates:
  kind: regex
  surface: whole
  pattern: 'marker'
  exclude_pattern: 'approved'
  paths: [src, crates, vendor]
  globs: ['!*.txt']
  paths_exempt: [generated]
---
Café
"""
NOT_UTF8_SOURCE = b"// marker caf\xe9\n"

Case = dict[str, object]
Reply = dict[str, object]


def git(repo: Path, *args: str, content: str = "") -> str:
    return subprocess.run(
        ("git", "-C", str(repo), *args), input=content, text=True, capture_output=True, check=True,
    ).stdout.strip()


def blob(repo: Path, content: bytes) -> str:
    return subprocess.run(
        ("git", "-C", str(repo), "hash-object", "-w", "--stdin"), input=content, capture_output=True, check=True,
    ).stdout.decode().strip()


def tree_with(repo: Path, tree: str, path: str, entry: str) -> str:
    """Return the tree with one entry added or replaced at the slash-separated path."""
    name, _, rest = path.partition("/")
    lines = [line for line in git(repo, "ls-tree", tree).splitlines() if not line.endswith(f"\t{name}")]
    if rest:
        entry = f"040000 tree {tree_with(repo, git(repo, 'rev-parse', f'{tree}:{name}'), rest, entry)}"
    return git(repo, "mktree", "--missing", content="\n".join([*lines, f"{entry}\t{name}"]) + "\n")


def commit_with(repo: Path, parent: str, path: str, entry: str, message: str) -> str:
    return git(repo, "commit-tree", tree_with(repo, parent, path, entry), "-p", parent, "-m", message)


def write(root: Path, relative: str, content: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_bytes(content.encode())


def commit(repo: Path, message: str) -> str:
    _ = git(repo, "add", "-f", ".")
    _ = git(repo, "commit", "-qm", message)
    return git(repo, "rev-parse", "HEAD")


def source_tips(repo: Path) -> tuple[str, str]:
    for key, value in (
        ("user.name", "Sites Test"),
        ("user.email", "sites@example.invalid"),
        ("core.hooksPath", "/dev/null"),
        ("commit.gpgsign", "false"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)
    write(repo, "Cargo.toml", '[workspace]\nmembers = ["crates/*"]\n')
    for folder, package in (("alpha", "focus"), ("beta", "beta")):
        write(repo, f"crates/{folder}/Cargo.toml", f'[package]\nname = "{package}"\n')
        write(repo, f"crates/{folder}/src/lib.rs", "// marker member\n")
    for path, content in {
        "src/a.rs": "// lead\r\n  marker marker\r\nmarker approved\r\n",
        "src/z.rs": "// marker duplicate source\n",
        "src/zz.rs": "// marker duplicate source\n",
        "src/separator.rs": "\x1fmarker unit separator\x1f\n",
        "src/notes.md": "marker café\nMARKER capitalized\n",
        "src/committed-ignore.md": "marker committed ignore\n",
        "src/tool-ignore.md": "marker tool ignore\n",
        "src/next-ignore.md": "marker next ignore\n",
        "src/fmt-ignore.md": "marker format ignore\n",
        "src/generated/output.rs": "marker exempt path\n",
        "src/outside-glob.txt": "marker outside glob\n",
        "vendor/package/src/lib.rs": "marker in vendor\n",
        "outside.rs": "marker outside roots\n",
        "src/.ignore": "committed-ignore.md\n",
        "src/.myignore": "tool-ignore.md\n",
        "rustfmt.toml": 'ignore = ["src/fmt-ignore.md"]\n',
    }.items():
        write(repo, path, content)
    synced = commit(repo, "Commit the listed source")
    write(repo, "src/.ignore", "next-ignore.md\n")
    # Python fnmatch drops the reversed range: `[z-a]` matches nothing and `[!z-a]` matches any character.
    # A dropped range before `!` makes it a negation: `[z-a!x]` is `[^x]` and `[z-a!]` is any character.
    write(repo, "src/memo.md", "marker memo\n")
    write(repo, "src/list.md", "marker list\n")
    write(
        repo,
        "rustfmt.toml",
        'ignore = ["src/fmt-ignore.md", "src/[z-a]*", "src/[!z-a]otes.md", "src/[z-a!x]emo.md", "src/[z-a!]ist.md"]\n',
    )
    next_tip = commit(repo, "Change the committed ignore")
    write(repo, "src/.ignore", "a.rs\n")
    write(repo, "rustfmt.toml", 'ignore = ["src/notes.md"]\n')
    write(repo, "src/a.rs", "marker dirty\n")
    write(repo, "src/untracked.rs", "marker untracked\n")
    return synced, next_tip


def write_catalog(root: Path, repo: Path) -> None:
    style = root / "style"
    write(style, "rust/marker.md", MARKER_RULE.replace("\n", "\r\n"))
    write(style, "rust/types.md", "---\ntier: B\nfamily: types\ncandidates:\n"
          + "  kind: struct_fields\n  surface: whole\n  field_type_pattern: '^String$'\n---\n# Types\n")
    write(style, "rust/filter.md", "---\ntier: B\nfamily: names\npre_filter: marker\n---\n# Filter\n")
    write(style, "rust/format.md", "---\ntier: A\nmechanism: rustfmt\n---\n# Format\n")
    write(style, "rust/review.md", "---\ntier: C\nfamily: names\n---\n# Review\n")
    write(style, "rust/judgment.md", "---\ntier: E\n---\n# Judgment\n")
    write(repo, LOCAL.removeprefix("local:"), MARKER_RULE)
    write(root / "checkout", "Cargo.toml", '[workspace]\nmembers = ["crates/beta"]\n')
    write(root / "checkout", "crates/beta/Cargo.toml", '[package]\nname = "focus"\n')
    write(root / "checkout", LOCAL.removeprefix("local:"), MARKER_RULE)
    write(root / "python-exclusion", LOOKBEHIND.removeprefix("local:"),
          MARKER_RULE.replace("exclude_pattern: 'approved'", r"exclude_pattern: '(?<=marker )approved\Z'"))
    for directory, rule in (
        ("bad-pattern", MARKER_RULE.replace("pattern: 'marker'", "pattern: '('")),
        ("bad-exclusion", MARKER_RULE.replace("exclude_pattern: 'approved'", "exclude_pattern: '('")),
        ("bad-catalog", MARKER_RULE.replace("surface: whole", "surface: invalid")),
        ("bad-tier", "---\ntier: Z\nfamily: names\ncandidates:\n  kind: regex\n  pattern: 'marker'\n---\n# Tier\n"),
    ):
        write(root / directory, "docs/style/bad.md", rule)


def seed_ledger(repo: Path, path: Path, tip: str, version: int = 51) -> None:
    if version == max(MIGRATIONS):
        Ledger(repo, path).close()
    else:
        with closing(sqlite3.connect(path)) as connection, connection:
            migrations = "\n".join(
                migration for number, migration in sorted(MIGRATIONS.items()) if number <= version
            )
            _ = connection.executescript(
                "BEGIN;\nCREATE TABLE schema_version (version INTEGER NOT NULL);\n" + migrations
            )
            _ = connection.execute("INSERT INTO schema_version VALUES (?)", (version,))
    with closing(sqlite3.connect(path)) as connection, connection:
        _ = connection.execute("INSERT INTO sync (id, tip, synced_at) VALUES (1, ?, ?)", (tip, STAMP))


def copy_ledger(source: Path, target: Path) -> None:
    with closing(sqlite3.connect(source)) as original, closing(sqlite3.connect(target)) as copy:
        original.backup(copy)


def set_synced_commit(ledger: Path, tip: str) -> None:
    with closing(sqlite3.connect(ledger)) as connection, connection:
        _ = connection.execute("UPDATE sync SET tip = ?", (tip,))


def site_id(rule: str, pin: str, path: str, blob_id: str, line: int, text: str) -> str:
    identity = json.dumps([rule, pin, path, blob_id, line, text], ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(identity.encode()).hexdigest()


def listed(ledger: Ledger, checkout: Path, rule: str) -> tuple[ListedSite, ...]:
    with ledger.rules_from_checkout(checkout):
        return ledger.listed_sites(rule)


def reply(ledger: Ledger, checkout: Path, rule: str, unavailable: str = "") -> Reply:
    definition = next(definition for definition in rules.load_rules(checkout) if definition.rule_id == rule)
    tip = ledger.synced_commit()
    envelope: Reply = {
        "schema": 1,
        "rule": rule,
        "observed": {"synced_tip": tip, "rule_pin": definition.blob, "ledger_revision": "absent"},
    }
    if unavailable:
        return {**envelope, "status": "unavailable", "reason": unavailable}
    rows: list[dict[str, object]] = []
    for site in listed(ledger, checkout, rule):
        blob_id = git(ledger.repo, "rev-parse", f"{tip}:{site.path}")
        rows.append({
            "file": site.path, "line": site.line,
            "site_id": site_id(rule, definition.blob, site.path, blob_id, site.line, site.text),
        })
    return {**envelope, "status": "listed", "sites": rows}


def success(name: str, ledger: str, checkout: str, expected: Reply,
            *, config: str = "", ignore: str = "") -> Case:
    return {
        "name": name, "ledger": ledger, "checkout": checkout, "rule": expected["rule"],
        "settings": {"kind": "configured", "config": config, "ignore": ignore} if config else {"kind": "default"},
        "expected": {"kind": "reply", "value": expected},
    }


def python_refuses(root: Path, ledger: str, checkout: str, rule: str,
                   exception: type[Exception], python_reason: str) -> None:
    oracle_path = root / "failure-oracle.sqlite"
    copy_ledger(root / ledger, oracle_path)
    try:
        with Ledger(root / "repo", oracle_path) as oracle:
            _ = listed(oracle, root / checkout, rule)
    except exception as error:
        diagnostic = error.stderr if isinstance(error, GitReadFailed) else str(error)
        assert python_reason in diagnostic, diagnostic
    else:
        raise AssertionError(f"Python listed sites of {rule} with checkout {checkout}")
    oracle_path.unlink()


def failure(root: Path, name: str, ledger: str, checkout: str, rule: str,
            exception: type[Exception], python_reason: str, fragments: list[str]) -> Case:
    python_refuses(root, ledger, checkout, rule, exception, python_reason)
    return {
        "name": name, "ledger": ledger, "checkout": checkout, "rule": rule,
        "settings": {"kind": "default"}, "expected": {"kind": "failure", "fragments": fragments},
    }


def regex_replies(root: Path, tip: str, next_tip: str) -> list[Case]:
    repo = root / "repo"
    fresh = root / "fresh ledger.sqlite"
    oracle_path = root / "oracle.sqlite"
    seed_ledger(repo, fresh, tip)
    copy_ledger(fresh, oracle_path)
    seed_ledger(repo, root / "schema 47.sqlite", tip, 47)
    copy_ledger(fresh, root / "ignore change.sqlite")
    set_synced_commit(root / "ignore change.sqlite", next_tip)
    with Ledger(repo, oracle_path) as oracle:
        answer = reply(oracle, repo, REGEX)
        sites = listed(oracle, repo, REGEX)
        coordinates = [(site.path, site.line) for site in sites]
        assert coordinates == [
            ("crates/alpha/src/lib.rs", 1), ("crates/beta/src/lib.rs", 1), ("src/a.rs", 2),
            ("src/next-ignore.md", 1), ("src/notes.md", 1), ("src/separator.rs", 1), ("src/tool-ignore.md", 1),
            ("src/z.rs", 1), ("src/zz.rs", 1),
        ], coordinates
        separator = [site.text for site in sites if site.path == "src/separator.rs"]
        assert separator == ["marker unit separator"], separator
        lookbehind = listed(oracle, root / "python-exclusion", LOOKBEHIND)
        assert [(site.path, site.line) for site in lookbehind] == coordinates, lookbehind
        cases = [
            success("committed regex rows with a dirty checkout", fresh.name, "repo", answer),
            success("schema 47", "schema 47.sqlite", "repo", answer),
            success("repository catalog member", fresh.name, "repo", reply(oracle, repo, LOCAL)),
            success("external catalog member differs from committed member", fresh.name, "checkout",
                    reply(oracle, root / "checkout", LOCAL)),
            success("exclusion uses the Python re dialect", fresh.name, "python-exclusion",
                    reply(oracle, root / "python-exclusion", LOOKBEHIND)),
        ]
        local = listed(oracle, root / "checkout", LOCAL)
        assert [(site.path, site.crate) for site in local] == [
            ("crates/beta/src/lib.rs", "crates/beta"),
        ], local
    # Deliberate difference from Python _refresh_candidate_sites: an invalid rule does not block another rule.
    python_refuses(root, fresh.name, "bad-exclusion", REGEX, LedgerError, "unterminated subpattern")
    cases.append(success("an invalid rule does not block another rule", fresh.name, "bad-exclusion", answer))
    set_synced_commit(oracle_path, next_tip)
    with Ledger(repo, oracle_path) as oracle:
        changed = reply(oracle, repo, REGEX)
        paths = {site.path for site in listed(oracle, repo, REGEX)}
        assert "src/committed-ignore.md" in paths and "src/next-ignore.md" not in paths, paths
        assert {"src/notes.md", "src/memo.md", "src/list.md"}.isdisjoint(paths), paths
        cases.append(success("changed committed ignore with reversed ranges", "ignore change.sqlite", "repo", changed))
    return cases


def configured_replies(root: Path) -> list[Case]:
    repo = root / "repo"
    config = root / "rg.conf"
    ignore = root / "rg.ignore"
    os.environ["RIPGREP_CONFIG_PATH"] = str(config)
    oracle_path = root / "configured oracle.sqlite"
    copy_ledger(root / "fresh ledger.sqlite", oracle_path)
    cases: list[Case] = []
    try:
        with Ledger(repo, oracle_path) as oracle:
            for name, excluded, case_flag in (
                ("rg uses configured case and ignore options", "src/tool-ignore.md", "--ignore-case\n"),
                ("rg reads changed configuration bytes", "src/tool-ignore.md", ""),
                ("rg reads changed external ignore bytes", "src/notes.md", ""),
            ):
                settings = f"--vimgrep\n{case_flag}--ignore-file\n{ignore}\n"
                write(root, config.name, settings)
                write(root, ignore.name, excluded + "\n")
                answer = reply(oracle, repo, REGEX)
                rows = listed(oracle, repo, REGEX)
                assert len([row for row in rows if row.path == "src/a.rs" and row.line == 2]) == 2, rows
                assert excluded not in {row.path for row in rows}, rows
                assert any(row.path == "src/notes.md" and row.line == 2 for row in rows) == bool(case_flag), rows
                cases.append(success(name, "fresh ledger.sqlite", "repo", answer,
                                     config=settings, ignore=excluded + "\n"))
            relative = "--ignore-file=./src/.myignore\n"
            write(root, config.name, relative)
            write(root, ignore.name, "")
            answer = reply(oracle, repo, REGEX)
            rows = listed(oracle, repo, REGEX)
            assert "src/tool-ignore.md" not in {row.path for row in rows}, rows
            cases.append(success("rg reads a relative tree ignore file", "fresh ledger.sqlite", "repo", answer,
                                 config=relative))
    finally:
        del os.environ["RIPGREP_CONFIG_PATH"]
    return cases


def legacy_replies(root: Path, tip: str) -> list[Case]:
    repo = root / "repo"
    oracle_path = root / "legacy oracle.sqlite"
    copy_ledger(root / "fresh ledger.sqlite", oracle_path)
    with Ledger(repo, oracle_path) as oracle:
        fresh = reply(oracle, repo, REGEX)
    with closing(sqlite3.connect(oracle_path)) as connection, connection:
        _ = connection.execute("DELETE FROM refill_candidates")
        _ = connection.execute("DELETE FROM listed_sites WHERE rule = ? AND path != 'src/z.rs'", (REGEX,))
    copy_ledger(oracle_path, root / "legacy.sqlite")
    with Ledger(repo, oracle_path) as oracle:
        legacy = reply(oracle, repo, REGEX)
        stored = listed(oracle, repo, REGEX)
        assert [row.path for row in stored] == ["src/z.rs"] and fresh != legacy, stored
    schema_47 = root / "schema 47 legacy.sqlite"
    seed_ledger(repo, schema_47, tip, 47)
    with closing(sqlite3.connect(schema_47)) as connection, connection:
        _ = connection.execute("ATTACH DATABASE ? AS legacy", (str(root / "legacy.sqlite"),))
        _ = connection.execute(
            "INSERT INTO listed_rules (rule, rule_blob, tip) SELECT rule, rule_blob, tip FROM legacy.listed_rules")
        _ = connection.execute(
            "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
            + "SELECT rule, rule_blob, tip, path, line, text FROM legacy.listed_sites")
    copy_ledger(schema_47, root / "schema 47 oracle.sqlite")
    with Ledger(repo, root / "schema 47 oracle.sqlite") as oracle:
        migrated = reply(oracle, repo, REGEX)
        assert migrated == legacy, migrated
    with closing(sqlite3.connect(oracle_path)) as connection, connection:
        _ = connection.execute("DELETE FROM listed_rules WHERE rule = 'global:rust/types.md'")
    copy_ledger(oracle_path, root / "incomplete markers.sqlite")
    with Ledger(repo, oracle_path) as oracle:
        refreshed = reply(oracle, repo, REGEX)
        assert refreshed == fresh, refreshed
    return [
        success("pinned legacy marker preserves listed rows", "legacy.sqlite", "repo", legacy),
        success("schema 47 legacy marker preserves listed rows", schema_47.name, "repo", migrated),
        success("incomplete legacy markers regenerate listed rows", "incomplete markers.sqlite", "repo", refreshed),
    ]


def failure_replies(root: Path, tip: str) -> list[Case]:
    repo = root / "repo"
    missing = "f" * 40
    broken = commit_with(repo, tip, "missing.rs", f"100644 blob {missing}", "Add an unavailable object")
    not_utf8 = commit_with(repo, tip, "src/latin.rs", f"100644 blob {blob(repo, NOT_UTF8_SOURCE)}",
                           "Add a matching source that is not UTF-8")
    for ledger, commit_id in (("missing.sqlite", broken), ("not utf8.sqlite", not_utf8)):
        copy_ledger(root / "fresh ledger.sqlite", root / ledger)
        set_synced_commit(root / ledger, commit_id)
    fresh = "fresh ledger.sqlite"
    local = "local:docs/style/bad.md"
    return [
        failure(root, "invalid ripgrep pattern", fresh, "bad-pattern", local, LedgerError,
                "unclosed group", ["candidates regex for", "rg exit", "unclosed group"]),
        failure(root, "invalid exclusion pattern", fresh, "bad-exclusion", local, LedgerError,
                "unterminated subpattern", [local, "unterminated subpattern"]),
        failure(root, "invalid rule frontmatter", fresh, "bad-catalog", local, rules.RuleValidationError,
                "candidates.surface must be whole or partial",
                ["docs/style/bad.md", "candidates.surface must be whole or partial"]),
        failure(root, "tier is checked before the candidates surface", fresh, "bad-tier", local,
                rules.RuleValidationError, "tier must be A, B, C, D or E",
                ["docs/style/bad.md", "tier must be A, B, C, D or E"]),
        failure(root, "unknown rule", fresh, "repo", "global:rust/unknown.md", LedgerError,
                "unknown rule", ["unknown rule", "global:rust/unknown.md"]),
        failure(root, "missing committed blob", "missing.sqlite", "repo", REGEX, GitReadFailed,
                "candidate input object is unavailable", ["candidate input object is unavailable", missing]),
        # Ripgrep prints the exact bytes of the matching line, so neither reader decodes them.
        failure(root, "matching source that is not UTF-8", "not utf8.sqlite", "repo", REGEX, UnicodeDecodeError,
                "can't decode byte 0xe9", ["cannot read the rg configuration or output as UTF-8"]),
    ]


def main(root: Path) -> None:
    repo = root / "repo"
    _ = os.environ.pop("RIPGREP_CONFIG_PATH", None)
    tip, next_tip = source_tips(repo)
    write_catalog(root, repo)
    definition = next(rule for rule in rules.load_rules(repo) if rule.rule_id == REGEX)
    assert definition.candidates is not None
    working = candidate_generators.gen_regex(definition.candidates, repo)
    assert any(row.file == "src/untracked.rs" for row in working.candidates), working
    cases = regex_replies(root, tip, next_tip) + configured_replies(root) + legacy_replies(root, tip)
    with Ledger(repo, root / "oracle.sqlite") as oracle:
        for rule, reason in (
            ("global:rust/types.md", "unsupported_generator"),
            ("global:rust/filter.md", "prefilter_requires_shared_membership"),
        ):
            cases.append(success(reason, "ignore change.sqlite", "repo", reply(oracle, repo, rule, reason)))
        for tier, rule in (
            ("A", "global:rust/format.md"),
            ("C", "global:rust/review.md"),
            ("E", "global:rust/judgment.md"),
        ):
            assert listed(oracle, repo, rule) == (), rule
            cases.append(success(f"tier {tier} rule lists no sites", "ignore change.sqlite", "repo",
                                 reply(oracle, repo, rule)))
    print(json.dumps(cases + failure_replies(root, tip)))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
