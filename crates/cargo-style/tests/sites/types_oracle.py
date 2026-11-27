"""Read the expected type and trait sites from the pinned Python ledger."""

from __future__ import annotations

import json
import sqlite3
import sys
from contextlib import closing
from pathlib import Path
from typing import TypedDict

from oracle import Reply, commit, copy_ledger, git, listed, reply, seed_ledger, write
from scripts.style.style_ledger import Ledger


AFFIXES = "global:rust/field-affixes.md"
VARIANTS = "global:rust/variant-stutter.md"
STUTTER = "global:rust/field-type-stutter.md"
FIELDS = "global:rust/struct-fields.md"
TRAITS = "global:rust/single-impl-traits.md"
MEMBER_FIELDS = "local:docs/focus/style/fields.md"
PREFILTER = "global:rust/prefilter.md"
FRESH_LEDGER = "types.sqlite"
ALPHA_TYPES = "crates/alpha/src/types.rs"
BETA_TYPES = "crates/beta/src/types.rs"
TYPE_RULES = (AFFIXES, VARIANTS, STUTTER, FIELDS, TRAITS, MEMBER_FIELDS)


class TypeSitesCase(TypedDict):
    name: str
    rule: str
    ledger: str
    expected: Reply


TYPE_SOURCE = """#[cfg(any())]
pub(crate) struct HttpServer<'a, const N: usize>
where
    [(); { N }]: Sized,
{
    http_read_café: Option<&'a str>,
    http_write_café: Option<&'a str>,
    http_retries_café: usize,
}
pub struct Choice {
    pub r#choice: Choice,
    choice_valid: bool,
}
pub struct Other {
    option: Option<String>,
}
pub struct Rates {
    read_ms: u64,
    write_ms: u64,
}
pub struct Mixed {
    read_first: bool,
    read_second: bool,
    independent: bool,
}
pub enum HTTPRequest {
    HTTPRequestError,
    HttpOk,
    Other,
}
#[cfg(any())] struct GhostAttribute {
    ghost_one: Option<String>,
    ghost_two: Option<String>,
}
struct Inline { inline: bool } struct GhostSecond {
    ghost_one: Option<String>,
    ghost_two: Option<String>,
}
pub struct CharGuard<const N: usize = { b'}' as usize + '{' as usize }> {
    char_guard_left: u8,
    char_guard_right: u8,
}
"""

MASKED_SOURCE = r'''/*
    /* Nested comment with a brace } */
struct GhostComment {
    ghost_one: Option<String>,
    ghost_two: Option<String>,
}
*/
const RAW: &str = r##" contains "#
struct GhostRaw {
    ghost_one: Option<String>,
    ghost_two: Option<String>,
}
impl Once for Fake {}
"##;
const RAW_BYTES: &[u8] = br#"raw bytes
impl Once for Fake {}
"#;
const PLAIN: &str = "contains an escaped quote \"
struct GhostString {
    ghost_one: Option<String>,
    ghost_two: Option<String>,
}
impl Once for Fake {}
";
const BYTES: &[u8] = b"byte string
impl Once for Fake {}
";
// impl Once for Fake {}
/* impl Once for Fake {} */
pub struct Visible {
    visible_left: u8,
    visible_right: u8,
}
'''

FILTERED_SOURCE = """struct Filtered {
    filter_left: Option<String>,
    filter_right: Option<String>,
}
"""


def write_source(repo: Path) -> str:
    for key, value in (
        ("user.name", "Type Sites Test"),
        ("user.email", "type-sites@example.invalid"),
        ("core.hooksPath", "/dev/null"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("maintenance.auto", "false"),
    ):
        _ = git(repo, "config", key, value)
    write(repo, "Cargo.toml", '[workspace]\nmembers = ["crates/*"]\n')
    for folder, package in (("alpha", "focus"), ("beta", "beta")):
        write(repo, f"crates/{folder}/Cargo.toml", f'[package]\nname = "{package}"\n')
    write(repo, ALPHA_TYPES, TYPE_SOURCE.replace("\n", "\r\n"))
    write(repo, BETA_TYPES, TYPE_SOURCE.replace("\n", "\r\n"))
    write(repo, "crates/alpha/src/lib.rs", """pub trait Shared {}
pub trait Coupled {}
pub(crate) unsafe trait Once {}
pub trait Zero {}
pub trait IgnoredImpl {}
#[cfg(any())]
pub trait Conditional {}
pub trait First {} pub trait SameLine {}
""")
    write(repo, "crates/beta/src/lib.rs", """pub trait Shared {}
pub trait Coupled {}
impl alpha::Shared for Worker {}
impl alpha::Coupled for Worker {}
impl beta::Coupled for Other {}
impl alpha::IgnoredImpl for Worker {}
impl<T> alpha::Once
    for Box<T>
{}
""")
    write(repo, "src/masked.rs", MASKED_SOURCE)
    for name, opening in (
        ("comment", "/* outer /* inner */\n"),
        ("string", 'const OPEN: &str = "open\\\n'),
        ("raw", 'const OPEN: &str = r##"wrong closer: "#\n'),
    ):
        visible = "struct Before {\n    before_left: u8,\n    before_right: u8,\n}\n"
        ghost = "struct GhostUnclosed {\n    ghost_one: u8,\n    ghost_two: u8,\n}\n"
        write(repo, f"src/open_{name}.rs", visible + opening + ghost)
    for path in ("src/.hidden/fields.rs", "target/fields.rs", "vendor/fields.rs", "src/fields.md"):
        write(repo, path, FILTERED_SOURCE)
    write(repo, "src/exclude/fields.rs", FILTERED_SOURCE + "impl IgnoredImpl for Other {}\n")
    write(repo, "rustfmt.toml", 'ignore = ["src/exclude"]\n')
    tip = commit(repo, "Save the generator input")
    write(repo, ALPHA_TYPES, FILTERED_SOURCE)
    return tip


def write_rules(root: Path, repo: Path) -> None:
    for rule, kind in (
        (AFFIXES, "field_affixes"),
        (VARIANTS, "enum_variant_stutter"),
        (STUTTER, "field_type_stutter"),
        (TRAITS, "single_impl_traits"),
    ):
        write(root / "style", rule.removeprefix("global:"),
              f"---\ntier: B\nfamily: types\ncandidates:\n  kind: {kind}\n  surface: whole\n---\nType sites\n")
    fields = """---
tier: B
family: types
candidates:
  kind: struct_fields
  surface: partial
  uncovered: Other field types need a review.
  structs: [HttpServer, Choice]
  field_type_pattern: '^Option<|^Choice$'
---
Selected field types
"""
    write(root / "style", FIELDS.removeprefix("global:"), fields)
    write(repo, MEMBER_FIELDS.removeprefix("local:"), fields)
    write(root / "style", PREFILTER.removeprefix("global:"),
          "---\ntier: B\nfamily: types\npre_filter: NoSiteMatchesThis\n---\nPrefilter only\n")


def case(name: str, rule: str, ledger: str, expected: Reply) -> TypeSitesCase:
    return {"name": name, "rule": rule, "ledger": ledger, "expected": expected}


def check_fixture(oracle: Ledger, repo: Path) -> None:
    affixes = listed(oracle, repo, AFFIXES)
    duplicate = [(row.path, row.line) for row in affixes if row.text.startswith("struct HttpServer:")]
    assert duplicate == [(ALPHA_TYPES, 2), (ALPHA_TYPES, 2), (BETA_TYPES, 2), (BETA_TYPES, 2)], duplicate
    assert {row.path for row in affixes} == {
        ALPHA_TYPES, BETA_TYPES, "src/masked.rs", "src/open_comment.rs", "src/open_raw.rs", "src/open_string.rs",
    }, affixes
    assert all(row.text.startswith(("struct HttpServer:", "struct CharGuard:", "struct Choice:",
                                    "struct Visible:", "struct Before:")) for row in affixes), affixes
    assert any(row.text.startswith("struct CharGuard:") for row in affixes), affixes
    variants = listed(oracle, repo, VARIANTS)
    variant_lines = [index for index, line in enumerate(TYPE_SOURCE.splitlines(), 1)
                     if line.strip() in ("HTTPRequestError,", "HttpOk,")]
    assert [row.line for row in variants if row.path == ALPHA_TYPES] == variant_lines, variants
    stutter = listed(oracle, repo, STUTTER)
    assert any("field `choice_valid`" in row.text for row in stutter), stutter
    assert all("field `choice`" not in row.text for row in stutter), stutter
    fields = listed(oracle, repo, FIELDS)
    local_fields = listed(oracle, repo, MEMBER_FIELDS)
    assert {row.path for row in fields} == {ALPHA_TYPES, BETA_TYPES}, fields
    assert [row.text for row in local_fields] == [
        "struct HttpServer: field `http_read_café: Option<&'a str>`",
        "struct HttpServer: field `http_write_café: Option<&'a str>`",
        "struct Choice: field `choice: Choice`",
    ], local_fields
    assert {row.path for row in local_fields} == {ALPHA_TYPES}, local_fields
    traits = listed(oracle, repo, TRAITS)
    assert [row.text for row in traits] == [
        "trait Shared: 1 impl(s) project-wide",
        "trait Once: 1 impl(s) project-wide",
        "trait Zero: 0 impl(s) project-wide",
        "trait Conditional: 0 impl(s) project-wide",
        "trait First: 0 impl(s) project-wide",
        "trait Shared: 1 impl(s) project-wide",
    ], traits


def legacy_cases(root: Path, repo: Path, tip: str) -> list[TypeSitesCase]:
    legacy = root / "legacy types.sqlite"
    with closing(sqlite3.connect(legacy)) as connection, connection:
        _ = connection.execute("DELETE FROM refill_candidates")
        _ = connection.execute(
            "DELETE FROM listed_sites WHERE rule = ? AND (path != ? OR text NOT LIKE ?)",
            (AFFIXES, ALPHA_TYPES, "%shared suffix%"),
        )
    with Ledger(repo, legacy) as oracle:
        legacy_reply = reply(oracle, repo, AFFIXES)
        saved = listed(oracle, repo, AFFIXES)
        assert len(saved) == 1 and saved[0].path == ALPHA_TYPES and "shared suffix" in saved[0].text, saved

    old = root / "schema 47 types.sqlite"
    seed_ledger(repo, old, tip, 47)
    with closing(sqlite3.connect(old)) as connection, connection:
        _ = connection.execute("ATTACH DATABASE ? AS legacy", (str(legacy),))
        _ = connection.execute(
            "INSERT INTO listed_rules (rule, rule_blob, tip) SELECT rule, rule_blob, tip FROM legacy.listed_rules")
        _ = connection.execute(
            "INSERT INTO listed_sites (rule, rule_blob, tip, path, line, text) "
            + "SELECT rule, rule_blob, tip, path, line, text FROM legacy.listed_sites")
    migrated = root / "migrated types.sqlite"
    copy_ledger(old, migrated)
    with Ledger(repo, migrated) as oracle:
        migrated_reply = reply(oracle, repo, AFFIXES)
        assert migrated_reply == legacy_reply, migrated_reply
    return [
        case("schema 51 legacy marker preserves stored affixes", AFFIXES, legacy.name, legacy_reply),
        case("schema 47 legacy marker preserves stored affixes", AFFIXES, old.name, migrated_reply),
    ]


def main(root: Path) -> None:
    repo = root / "repo"
    tip = write_source(repo)
    write_rules(root, repo)
    seed_ledger(repo, root / FRESH_LEDGER, tip)
    reference = root / "legacy types.sqlite"
    copy_ledger(root / FRESH_LEDGER, reference)
    with Ledger(repo, reference) as oracle:
        check_fixture(oracle, repo)
        cases = [case(rule, rule, FRESH_LEDGER, reply(oracle, repo, rule)) for rule in TYPE_RULES]
        assert listed(oracle, repo, PREFILTER) == ()
        cases.append(case("prefilter with no matches lists no sites", PREFILTER, FRESH_LEDGER,
                          reply(oracle, repo, PREFILTER)))
    print(json.dumps(cases + legacy_cases(root, repo, tip)))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
