"""Read expected source sites from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import sys
from contextlib import closing
from pathlib import Path
from shutil import copyfile
from typing import TypedDict

from oracle import Reply, commit, commit_with, copy_ledger, git, listed, python_refuses, reply, seed_ledger, write
from scripts.style.git_process import GitReadFailed
from scripts.style.style_ledger import Ledger


ALLOWS = "global:rust/allows.md"
BOILERPLATE = "global:rust/test-allows.md"
OBSERVERS = "global:rust/observer-guards.md"
LITERALS = "global:rust/literals.md"
TEST_LITERALS = "global:rust/test-literals.md"
MEMBER = "local:docs/focus/style/literals.md"
FRESH_LEDGER = "source.sqlite"
ALPHA = "crates/alpha/src/values.rs"
BETA = "crates/beta/src/values.rs"


class SourceSitesCase(TypedDict):
    name: str
    rule: str
    ledger: str
    expected: Reply


ALLOW_SOURCE = """#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
fn used() { value.unwrap(); }
#[allow(dead_code, reason = "kept for an example")]
fn explained() {}
#[allow(
    clippy::panic,
)]
fn multi_line() {}
#[allow(
    clippy::expect_used,
    reason = "the example uses a fallback",
)]
fn explained_multi_line() {}
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod separate;
fn later_in_file() { value.unwrap(); }
// #[allow(dead_code)]
/* outer /* #[allow(dead_code)] */ comment */
"""

INCLUDE_SOURCE = """#[allow(clippy::expect_used)]
fn includes_other_code() { include!("other.rs"); }
"""

MACRO_SOURCE = """macro_rules! body { () => { item.unwrap() }; }
#[allow(clippy::unwrap_used)]
fn calls_macro() { body!(); }
"""

OBSERVER_SOURCE = """fn starts_now(event: On<Click>) { return; }
fn at_window_end<T>(
    event: Trigger<Click>,
) {
    let a = one();
    let b = two();
    let c = three();
    let d = four();
    if done() { return; }
}
fn after_window(event: On<Click>) {
    let a = one();
    let b = two();
    let c = three();
    let d = four();
    let e = five();
    return;
}
fn regular_fn(event: Click) { return; }
fn masked_guard(event: On<Click>) {
    // return;
    log("return");
}
"""

LITERAL_SOURCE = r'''#[doc = "attribute literal"]
pub(crate) const LIMIT: usize = 91;
const LABEL: &str = "constant string";
fn values() {
    let identity = (0, 1, 0.0, 1.0);
    let padded_zero = 0_0;
    let single = (2, 3, 4);
    let underscored = 2_000;
    let ratio = 3.75;
    let exponent = 7e-3;
    let array: [u8; 64] = build();
    let boundary = id42 + .88 + café²90;
    let decimal = １２;
    let plain = "café";
    let escaped = "a\"b";
    let two_strings = ("left", "right");
    let bytes = b"bytes";
    let raw = r##"a "# b
    #[allow(dead_code)] 91
    "##;
    let raw_bytes = br#"raw bytes"#;
    let chars = ('é', '\n', b'q', b'\x41');
    let empty_and_short = ("", "é");
    let fixed = ("Cargo.toml", "mod.rs", "crate", "src");
    let formatted = format!("message {}", value);
    assert_eq!(message, "assertion");
    let included = include_str!("file.txt");
    let other_include = include_bytes!("file.txt");
    let joined = concat!("one", "two");
    // let ignored = (88, "in a comment");
    /* let ignored = (99, "nested"); /* nested comment */ */
}
#[cfg(test)]
mod checks {
    const CHECK: u32 = 99;
    fn test_values() {
        let unused = 843;
        assert_eq!(got, 2_000);
        let below = got <= 2.75;
        let fits = matches!(got, 3e1);
        assert_eq!(got, 1);
        assert_eq!(got, "test string");
    }
}
#[cfg(test)]
mod tests;
fn after_external_test() { assert_eq!(got, 57); }
'''

TEST_SOURCE = """fn test_values() {
    assert_ne!(seen, 61);
    let unused = 63;
    if seen != 62 { check(); }
}
"""

ALLOWS_PATH = "src/allows.rs"
INCLUDE_PATH = "src/include.rs"
MACRO_PATH = "src/macro.rs"
OBSERVER_PATH = "src/observers.rs"
TEST_PATHS = ("src/integration_tests.rs", "tests/unit.rs")
SHARED_SOURCE = "fn check() { assert_eq!(actual, 42); }\n"
PRODUCTION_CHECK = "src/check.rs"
TEST_CHECK = "tests/check.rs"


def write_source(repo: Path) -> str:
    for key, value in (
        ("user.name", "Source Sites Test"),
        ("user.email", "source-sites@example.invalid"),
        ("core.hooksPath", "/dev/null"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)
    write(repo, "Cargo.toml", '[workspace]\nmembers = ["crates/*"]\n')
    for folder, package in (("alpha", "focus"), ("beta", "beta")):
        write(repo, f"crates/{folder}/Cargo.toml", f'[package]\nname = "{package}"\n')
    for path, source in (
        (ALLOWS_PATH, ALLOW_SOURCE),
        (INCLUDE_PATH, INCLUDE_SOURCE),
        (MACRO_PATH, MACRO_SOURCE),
        (OBSERVER_PATH, OBSERVER_SOURCE),
        (ALPHA, LITERAL_SOURCE.replace("\n", "\r\n")),
        (BETA, LITERAL_SOURCE.replace("\n", "\r\n")),
        *((path, TEST_SOURCE) for path in TEST_PATHS),
        (PRODUCTION_CHECK, SHARED_SOURCE),
        (TEST_CHECK, SHARED_SOURCE),
    ):
        write(repo, path, source)
    for path in (
        "examples/values.rs", "benches/values.rs", "build.rs", "src/generated/values.rs",
        "src/fmt_skip/values.rs", "src/.hidden/values.rs", "target/values.rs", "src/values.md",
    ):
        write(repo, path, 'fn excluded() { let value = (92, "excluded"); }\n')
    _ = (repo / "src/unreadable.rs").write_bytes(b'fn invalid() { let value = (94, "bad \xff"); }\n')
    write(repo, "rustfmt.toml", 'ignore = ["src/fmt_skip"]\n')
    tip = commit(repo, "Save the source sites input")
    write(repo, ALPHA, 'fn unstaged() { let value = (95, "unstaged"); }\n')
    return tip


def write_rules(root: Path, repo: Path) -> None:
    for rule, kind in (
        (ALLOWS, "allows_without_reason"),
        (BOILERPLATE, "test_allow_boilerplate"),
        (OBSERVERS, "observer_guards"),
        (TEST_LITERALS, "test_literals"),
    ):
        write(root / "style", rule.removeprefix("global:"),
              f"---\ntier: B\nfamily: values\ncandidates:\n  kind: {kind}\n  surface: whole\n---\nSource sites\n")
    literals = """---
tier: B
family: values
candidates:
  kind: literals
  paths_exempt: [generated]
"""
    write(root / "style", LITERALS.removeprefix("global:"), literals + "  surface: whole\n---\nLiteral sites\n")
    write(repo, MEMBER.removeprefix("local:"), literals + """  surface: partial
  uncovered: Other literals need review.
---
Member literal sites
""")


def line_of(source: str, marker: str) -> int:
    return next(number for number, line in enumerate(source.splitlines(), 1) if marker in line)


def check_allows(oracle: Ledger, repo: Path) -> None:
    rows = listed(oracle, repo, ALLOWS)
    assert [(row.path, row.line, row.text) for row in rows] == [
        (ALLOWS_PATH, 1, ALLOW_SOURCE.splitlines()[0]),
        (ALLOWS_PATH, 5, "#[allow("),
        (ALLOWS_PATH, line_of(ALLOW_SOURCE, "#[allow(clippy::unwrap_used, clippy::expect_used)]"),
         "#[allow(clippy::unwrap_used, clippy::expect_used)]"),
        (INCLUDE_PATH, 1, "#[allow(clippy::expect_used)]"),
        (MACRO_PATH, 2, "#[allow(clippy::unwrap_used)]"),
    ], rows
    stale = listed(oracle, repo, BOILERPLATE)
    assert [(row.path, row.line, row.text) for row in stale] == [
        (ALLOWS_PATH, 1, "strip clippy::expect_used — 0 matches of `.expect(` in scope"),
        (ALLOWS_PATH, 1, "strip clippy::panic — 0 matches of `panic!` in scope"),
        (ALLOWS_PATH, 5, "strip clippy::panic — 0 matches of `panic!` in scope"),
        (ALLOWS_PATH, 9, "strip clippy::expect_used — 0 matches of `.expect(` in scope"),
        (ALLOWS_PATH, 14, "strip clippy::expect_used — 0 matches of `.expect(` in scope"),
        (INCLUDE_PATH, 1, "strip clippy::expect_used — 0 matches of `.expect(` in scope"),
        (MACRO_PATH, 2, "strip clippy::unwrap_used — 0 matches of `.unwrap(` in scope"),
    ], stale


def check_literals(oracle: Ledger, repo: Path) -> None:
    lines = [line_of(LITERAL_SOURCE, marker) for marker in (
        "let padded_zero", "let underscored", "let ratio", "let exponent", "let decimal", "let plain", "let escaped",
        "let two_strings", "let bytes", "let raw =", "let raw_bytes", "fn after_external_test",
    )]
    literals = listed(oracle, repo, LITERALS)
    assert [(row.path, row.line) for row in literals] == [
        *((path, line) for path in (ALPHA, BETA) for line in lines),
        (ALLOWS_PATH, line_of(ALLOW_SOURCE, 'reason = "the example uses a fallback"')),
        (PRODUCTION_CHECK, 1),
        (INCLUDE_PATH, line_of(INCLUDE_SOURCE, "include!")),
        (OBSERVER_PATH, line_of(OBSERVER_SOURCE, 'log("return")')),
    ], literals
    local = listed(oracle, repo, MEMBER)
    assert [(row.path, row.line) for row in local] == [(ALPHA, line) for line in lines], local
    test_lines = [line_of(LITERAL_SOURCE, marker) for marker in ("assert_eq!(got, 2_000)", "let below", "let fits")]
    tests = listed(oracle, repo, TEST_LITERALS)
    assert [(row.path, row.line) for row in tests] == [
        *((path, line) for path in (ALPHA, BETA) for line in test_lines),
        *sorted([(TEST_CHECK, 1), *((path, line) for path in TEST_PATHS for line in (2, 4))]),
    ], tests


def case(name: str, rule: str, ledger: str, expected: Reply) -> SourceSitesCase:
    return {"name": name, "rule": rule, "ledger": ledger, "expected": expected}


def legacy_cases(root: Path, repo: Path, tip: str, legacy: Path) -> list[SourceSitesCase]:
    with closing(sqlite3.connect(legacy)) as connection, connection:
        _ = connection.execute("DELETE FROM refill_candidates")
        _ = connection.execute(
            "DELETE FROM listed_sites WHERE rule = ? AND (path != ? OR text NOT LIKE ?)",
            (LITERALS, ALPHA, "let raw_bytes =%"),
        )
    with Ledger(repo, legacy) as oracle:
        saved = listed(oracle, repo, LITERALS)
        assert [(row.path, row.line) for row in saved] == [(ALPHA, line_of(LITERAL_SOURCE, "let raw_bytes"))], saved
        legacy_reply = reply(oracle, repo, LITERALS)

    old = root / "schema 47 source.sqlite"
    seed_ledger(repo, old, tip, 47)
    with closing(sqlite3.connect(old)) as connection, connection:
        _ = connection.execute("ATTACH DATABASE ? AS legacy", (str(legacy),))
        _ = connection.execute(
            "INSERT INTO listed_rules (rule, rule_blob, tip) SELECT rule, rule_blob, tip FROM legacy.listed_rules")
        _ = connection.execute(
            "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
            + "SELECT rule, rule_blob, tip, path, line, text FROM legacy.listed_sites")
    migrated = root / "migrated source.sqlite"
    copy_ledger(old, migrated)
    with Ledger(repo, migrated) as oracle:
        migrated_reply = reply(oracle, repo, LITERALS)
        assert migrated_reply == legacy_reply, migrated_reply
    return [
        case("schema 51 preserves the legacy literal marker", LITERALS, legacy.name, legacy_reply),
        case("schema 47 preserves the legacy literal marker", LITERALS, old.name, migrated_reply),
    ]


def missing_blob_case(root: Path, repo: Path, tip: str) -> SourceSitesCase:
    missing = "f" * 40
    broken = commit_with(repo, tip, "src/missing.rs", f"100644 blob {missing}", "Add an unavailable object")
    staging = "missing staging.sqlite"
    ledger = "missing source.sqlite"
    reason = "candidate input object is unavailable"
    copy_ledger(root / FRESH_LEDGER, root / staging)
    with closing(sqlite3.connect(root / staging)) as connection:
        mode = connection.execute("PRAGMA journal_mode = WAL").fetchone()
        assert mode == ("wal",), mode
        with connection:
            _ = connection.execute("UPDATE sync SET tip = ?", (broken,))
        _ = copyfile(root / staging, root / ledger)
        _ = copyfile(root / (staging + "-wal"), root / (ledger + "-wal"))
        python_refuses(root, staging, "repo", LITERALS, GitReadFailed, reason)
    return case("literals refuses an unavailable object", LITERALS, ledger, {"refusal": [reason, missing]})


def main(root: Path) -> None:
    repo = root / "repo"
    tip = write_source(repo)
    write_rules(root, repo)
    seed_ledger(repo, root / FRESH_LEDGER, tip)
    legacy = root / "legacy source.sqlite"
    copy_ledger(root / FRESH_LEDGER, legacy)
    with Ledger(repo, legacy) as oracle:
        check_allows(oracle, repo)
        check_literals(oracle, repo)
        observers = listed(oracle, repo, OBSERVERS)
        assert [(row.path, row.line, row.text) for row in observers] == [
            (OBSERVER_PATH, 1, "observer fn starts_now opens with an early return"),
            (OBSERVER_PATH, 2, "observer fn at_window_end opens with an early return"),
        ], observers
        cases = [case(rule, rule, FRESH_LEDGER, reply(oracle, repo, rule))
                 for rule in (ALLOWS, BOILERPLATE, OBSERVERS, LITERALS, TEST_LITERALS, MEMBER)]
    print(json.dumps(cases + legacy_cases(root, repo, tip, legacy) + [missing_blob_case(root, repo, tip)]))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
