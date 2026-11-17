//! Compare every reader fact with the production Python readers.

use std::collections::BTreeMap;
use std::path::Path;

use cargo_berth_test_support::FixtureCommand as Command;
use cargo_berth_test_support::fixture_tempdir;
use serde_json::Value;
use serde_json::json;

mod support;

use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::git_input;
use support::output_with_input;
use support::parity_checkout;

const HEAD_TREE_REVISION: &str = "HEAD^{tree}";

const MODULES: &str = concat!(
    "mod lock;\n",
    "#[path = \"lock.rs\"] mod direct;\n",
    "#[path = \"lo\\x63k.rs\"] mod escaped;\n",
    "#[cfg_attr(all(), path = \"lock.rs\")] mod conditional;\n",
    "mod outer { #[path = \"child.rs\"] pub mod child; mod inner { mod leaf; } }\n",
    "fn work() { mod nested; }\n",
    "mod r#raw;\n",
    "make! { mod not_a_module; }\n",
);
const PATHS: &str = concat!(
    "mod lock;\n",
    "use crate::{lock::{self, Value as Alias, *}, other};\n",
    "use self::lock as alias;\n",
    "use super::{lock, sibling::*};\n",
    "use sample_library::lock::Value;\n",
    "fn work() { crate::lock::Value; self::lock::Value; super::lock::Value; lock::Value; alias::Value; }\n",
    "mod nested { use super::*; fn run() { lock::Value; } }\n",
    "// See [`crate::lock::Value`], self::lock and super::lock.\n",
    "const TEXT: &str = \"crate::lock::Value\";\n",
);
const ITEMS: &str = concat!(
    "#[cfg(test)]\n#[inline]\npub const fn r#work() { r#work(); }\n",
    "static mut STATE: u32 = 1;\nconst _: u32 = 2;\n",
    "struct Value<T> { value: T }\nenum Kind { First }\nunion Bits { value: u32 }\n",
    "type Number = u32;\nmod inline { pub fn child() { child(); } }\n",
    "trait Trait<T> { type Output; const LIMIT: u32 = 3; fn call(&self); fn defaulted() { defaulted(); } }\n",
    "impl<T> Value<T> { #[inline] fn value(&self) { value(); } }\n",
    "impl<T> path::Trait<T> for path::Value<T> where T: Copy { fn call(&self) { call(); } }\n",
    "fn host() { use path::Trait; impl Trait<u32> for Value<u32> { fn nested() { nested(); } } }\n",
    "impl dyn Trait for &Value<u32> { fn unreadable_owner() {} }\n",
    "impl Trait() for Value<u32> { fn unreadable_trait() {} }\n",
    "impl Value<u32> + Extra { fn unreadable_type() {} }\n",
    "impl<T: for<'a> Trait<'a>> Other<T> { fn higher_ranked() {} }\n",
);
const BODIES: &str = concat!(
    "fn same() { same(); let text = \"same\"; /* same */ let number = 1; }\n",
    "fn comments() { // comments\n /* comments */\n}\n",
    "fn unicode() { let text = \"é 😀\"; unicode(); }\n",
    "fn lines() {\r\n\r\n    lines();\r\n}\r\n",
    "fn separators() {\u{85}separators();\u{2028}}\n",
    "fn comments_with_cr() { // comment\rcomments_with_cr(); }\n",
    "fn comments_with_ff() { // comment\u{c}comments_with_ff(); }\n",
);
const BROKEN: &str = "fn cafe() { let café = 1; }\nfn unfinished() { /* no end";

fn reader_output(mut command: Command, request: &Value) -> TestResult<std::process::Output> {
    output_with_input(&mut command, &serde_json::to_vec(request)?)
}

fn output_lines(command: Command, request: &Value) -> TestResult<BTreeMap<String, Value>> {
    let output = reader_output(command, request)?;
    assert!(
        output.status.success(),
        "reader failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines = BTreeMap::new();
    for line in String::from_utf8(output.stdout)?.lines() {
        let value: Value = serde_json::from_str(line)?;
        let key = format!(
            "{} {} {} {}",
            value["blob"], value["fact"], value["path"], value["commit"]
        );
        assert!(
            lines.insert(key.clone(), value).is_none(),
            "duplicate fact: {key}"
        );
    }
    Ok(lines)
}

fn compare(request: &Value) -> TestResult<()> {
    let ParityCheckout::Enabled(root) = parity_checkout() else {
        return Ok(());
    };
    let rust = output_lines(Command::new(env!("CARGO_BIN_EXE_cargo-style")), request)?;
    let mut python = Command::new(root.join("scripts/lib/py"));
    python
        .current_dir(root)
        .args(["-m", "scripts.style.style_reader_oracle"]);
    let expected = output_lines(python, request)?;
    assert_eq!(
        rust.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "fact keys"
    );
    for (key, value) in expected {
        assert_eq!(rust[&key], value, "fact {key}");
    }
    Ok(())
}

fn write_code_identity_fixtures(repository: &Path) -> TestResult<()> {
    for (name, source) in [
        (
            "inner_docs.rs",
            "//! naïve 😀\n/*! outer /* nested */ doc */\nfn work() {}\n//! end\n",
        ),
        (
            "inner_docs_moved.rs",
            "fn work() {}\n//! naïve 😀\n/*! outer /* nested */ doc */\n//! end\n",
        ),
        (
            "outer_docs.rs",
            "/// outer docs\n/* ordinary /*! inner-looking */ comment */\nfn work() {}",
        ),
        (
            "literal_docs.rs",
            "const TEXT: &str = \"//! text\"; // ordinary\n//! \u{7f}\n",
        ),
    ] {
        std::fs::write(repository.join(name), source)?;
    }
    Ok(())
}

fn write_source_fixtures(repository: &Path) -> TestResult<()> {
    for (name, source) in [
        ("modules.rs", MODULES),
        ("paths.rs", PATHS),
        ("items.rs", ITEMS),
        ("bodies.rs", BODIES),
        (
            "equal_items.rs",
            "#[cfg(unix)] fn work() { work(); } #[cfg(windows)] fn work() { let n=1; work(); }",
        ),
        ("broken.rs", BROKEN),
        ("empty.rs", ""),
        ("comment.rs", "// crate::lock::Value\n"),
        (
            "uncertain.rs",
            "#[cfg_attr(all(), path = \"lock.rs\")] mod lock;",
        ),
        (
            "no_body.rs",
            "fn declared(); struct Tuple(u32); const EMPTY: u32 = 1;",
        ),
        ("incomplete.rs", "fn incomplete() { incomplete();"),
    ] {
        std::fs::write(repository.join(name), source)?;
    }
    for (name, source) in [
        ("not_utf8.rs", b"fn invalid() {}\xff".as_slice()),
        (
            "comment_not_utf8.rs",
            b"fn work() { work(); } // \xff\n".as_slice(),
        ),
        (
            "module_not_utf8.rs",
            b"mod lock; // \xed\xa0\x80\n".as_slice(),
        ),
    ] {
        std::fs::write(repository.join(name), source)?;
    }
    Ok(())
}

#[test]
fn all_facts_match_python_on_declarations_paths_bodies_and_unreadable_source() -> TestResult<()> {
    if matches!(parity_checkout(), ParityCheckout::Disabled) {
        return Ok(());
    }
    let directory = fixture_tempdir("cargo-style-parity-");
    let repository = directory.path();
    git(repository, &["init", "-q"])?;
    git(repository, &["config", "user.name", "Reader Test"])?;
    git(
        repository,
        &["config", "user.email", "reader@example.invalid"],
    )?;
    write_source_fixtures(repository)?;
    write_code_identity_fixtures(repository)?;
    std::fs::write(
        repository.join("byte_words.md"),
        b"shared \xff caf\xc3\xa9 under_score 42",
    )?;
    std::fs::write(repository.join("ignored.txt"), "fn excluded() {}")?;
    std::fs::write(
        repository.join("notes.md"),
        "# naïve_1\nWords in **Markdown**. T² ²shared xⅧ",
    )?;
    std::fs::create_dir(repository.join("folder"))?;
    std::fs::write(repository.join("folder/nested.md"), "nested words")?;
    std::fs::write(repository.join("folder.c"), "another entry")?;
    git(repository, &["add", "."])?;
    git(repository, &["commit", "-qm", "Add reader fixtures"])?;
    let first = git(repository, &["rev-parse", "HEAD"])?;
    std::fs::write(
        repository.join("history.rs"),
        "fn historical() { historical(); }",
    )?;
    git(repository, &["add", "."])?;
    git(repository, &["commit", "-qm", "Add another reader fixture"])?;
    let second = git(repository, &["rev-parse", "HEAD"])?;
    let moves = json!([
        {"old":"src/lock.rs", "new":"src/mutation_lock.rs", "file_module":{"crate":[]}, "parent":[], "library":{"name":"sample_library"}},
        {"old":"src/lock.rs", "new":"src/mutation_lock.rs", "file_module":"outside_crate", "parent":[], "library":{"name":"sample_library"}},
        {"old":"src/lock.rs", "new":"src/mutation_lock.rs", "file_module":"outside_crate", "parent":[], "library":"none"},
        {"old":"src/outer/lock.rs", "new":"src/outer/mutation_lock.rs", "file_module":{"crate":["outer","child"]}, "parent":["outer"], "library":"none"}
    ]);
    let mut queries: Vec<Value> = [
        "paths.rs",
        "modules.rs",
        "comment.rs",
        "uncertain.rs",
        "broken.rs",
        "not_utf8.rs",
    ]
    .into_iter()
    .map(|name| {
        let blob = git(repository, &["rev-parse", &format!("HEAD:{name}")])?;
        Ok(json!({"blob":blob, "path":"src/lib.rs", "moves":moves}))
    })
    .collect::<TestResult<Vec<_>>>()?;
    std::fs::write(repository.join("query_only.rs"), "crate::lock::Value;")?;
    let query_blob = git(repository, &["hash-object", "-w", "query_only.rs"])?;
    std::fs::remove_file(repository.join("query_only.rs"))?;
    queries.push(json!({"blob":query_blob, "path":"src/query_only.rs", "moves":moves}));
    compare(&json!({
        "repository":repository, "commits":[first,second],
        "facts":["tokens","lines","names","module_scan","declarations","name_sites","rust_code"],
        "module_sites":queries
    }))?;
    let tree = git(repository, &["rev-parse", HEAD_TREE_REVISION])?;
    compare(&json!({
        "repository":repository, "commits":[first,second,tree], "trees":true,
        "extensions":["rs","md"], "facts":["names"]
    }))?;
    let markdown = git(repository, &["rev-parse", "HEAD:notes.md"])?;
    compare(&json!({
        "repository":repository, "commits":[], "facts":["names"],
        "blobs":[markdown, "1111111111111111111111111111111111111111"]
    }))?;
    compare(&json!({
        "repository":repository, "commits":[first,second], "facts":["names"],
        "extensions":["rs","md"], "name_word_class":"ascii"
    }))?;
    compare_unreadable_paths(repository)
}

fn compare_unreadable_paths(repository: &Path) -> TestResult<()> {
    let blob = git_input(
        repository,
        &["hash-object", "-w", "--stdin"],
        b"fn raw_path_words() {}",
    )?;
    let mut entries = Vec::new();
    for path in [b"notes-\xff.txt".as_slice(), b"source-\xff.rs".as_slice()] {
        entries.extend_from_slice(format!("100644 blob {blob}\t").as_bytes());
        entries.extend_from_slice(path);
        entries.push(0);
    }
    let raw_paths = git_input(repository, &["mktree", "-z"], &entries)?;
    compare(&json!({
        "repository":repository, "commits":[raw_paths], "facts":["names"]
    }))?;
    let request = json!({
        "repository":repository, "commits":[raw_paths], "facts":[], "trees":true
    });
    let rust = reader_output(Command::new(env!("CARGO_BIN_EXE_cargo-style")), &request)?;
    let ParityCheckout::Enabled(root) = parity_checkout() else {
        return Ok(());
    };
    let mut python = Command::new(root.join("scripts/lib/py"));
    python
        .current_dir(root)
        .args(["-m", "scripts.style.style_reader_oracle"]);
    let expected = reader_output(python, &request)?;
    assert!(
        !expected.status.success(),
        "Python must reject a non-UTF-8 tree path"
    );
    assert_eq!(
        rust.status.success(),
        expected.status.success(),
        "tree path decoding"
    );
    Ok(())
}
