//! Audit a completed full-suite process report against `process_start_budget.json`.
//!
//! Entry point: set `CARGO_BERTH_PROCESS_COUNT_REPORT` to the reduced JSON report,
//! then run `verify.sh test cargo-berth --filter completed_process_report_obeys_budget
//! --no-cache`. Run this after collecting and reducing a complete cold suite,
//! including SETUP and hook descendants, before deleting its raw trace. An ordinary
//! test run checks the auditor's regressions; it cannot count its peer processes.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::json;

type StartsByOrigin = BTreeMap<String, u64>;

#[derive(Deserialize)]
struct CompletedProcessReport {
    suite_total:        u64,
    by_origin:          StartsByOrigin,
    setup:              StartsByOrigin,
    tests:              BTreeMap<String, StartsByOrigin>,
    unresolved_product: u64,
}

#[derive(Deserialize)]
struct SuiteProcessBudget {
    suite_total:     u64,
    by_origin:       StartsByOrigin,
    minimum_tests:   usize,
    expensive_tests: BTreeMap<String, u64>,
}

fn check_budget(
    report: &CompletedProcessReport,
    budget: &SuiteProcessBudget,
) -> Result<(), String> {
    if report.unresolved_product != 0 {
        return Err("unresolved product starts".to_owned());
    }
    if report.tests.len() < budget.minimum_tests {
        return Err("incomplete full-suite test coverage".to_owned());
    }
    if report.setup.values().sum::<u64>() == 0 {
        return Err("missing SETUP process coverage".to_owned());
    }
    let accounted = report
        .tests
        .values()
        .flat_map(BTreeMap::values)
        .sum::<u64>()
        + report.setup.values().sum::<u64>();
    if report.suite_total != report.by_origin.values().sum::<u64>()
        || report.suite_total != accounted
    {
        return Err("suite, origin and test totals disagree".to_owned());
    }
    for (name, ceiling) in &budget.expensive_tests {
        let row = report
            .tests
            .get(name)
            .ok_or_else(|| format!("missing expensive test: {name}"))?;
        check_ceiling(name, row.values().sum(), *ceiling)?;
    }
    check_ceiling("suite", report.suite_total, budget.suite_total)?;
    for (origin, starts) in &report.by_origin {
        check_ceiling(
            origin,
            *starts,
            budget.by_origin.get(origin).copied().unwrap_or(0),
        )?;
    }
    Ok(())
}

fn check_ceiling(name: &str, starts: u64, ceiling: u64) -> Result<(), String> {
    if starts > ceiling {
        Err(format!("{name}: {starts} starts exceed {ceiling}"))
    } else {
        Ok(())
    }
}

/// Check a fresh reduced report when explicitly supplied by the counting run.
/// Missing input means this ordinary test run has no completed measurement to audit.
#[test]
fn completed_process_report_obeys_budget() -> Result<(), Box<dyn std::error::Error>> {
    match env::var("CARGO_BERTH_PROCESS_COUNT_REPORT") {
        Ok(path) => {
            let report: CompletedProcessReport =
                serde_json::from_slice(&fs::read(Path::new(&path))?)?;
            let budget: SuiteProcessBudget =
                serde_json::from_str(include_str!("process_start_budget.json"))?;
            check_budget(&report, &budget)?;
            println!(
                "PASS: suite process budget ({} / {})",
                report.suite_total, budget.suite_total
            );
        },
        Err(env::VarError::NotPresent) => {},
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn measurement() -> Result<CompletedProcessReport, serde_json::Error> {
    serde_json::from_value(json!({
        "suite_total": 11,
        "by_origin": {"fixture_git": 5, "product_git": 6},
        "setup": {"fixture_git": 2, "product_git": 1},
        "tests": {"board": {"fixture_git": 1, "product_git": 3}, "gate": {"fixture_git": 2, "product_git": 2}},
        "unresolved_product": 0
    }))
}

fn budget() -> Result<SuiteProcessBudget, serde_json::Error> {
    serde_json::from_value(json!({
        "suite_total": 12, "by_origin": {"fixture_git": 6, "product_git": 7},
        "expensive_tests": {"board": 5}, "minimum_tests": 2
    }))
}

#[test]
fn extra_operation_calls_exceed_the_suite_budget() -> Result<(), Box<dyn std::error::Error>> {
    let mut report = measurement()?;
    let budget = budget()?;
    assert_eq!(check_budget(&report, &budget), Ok(()));
    report.tests.insert(
        "new_board_call".to_owned(),
        BTreeMap::from([("fixture_git".to_owned(), 1), ("product_git".to_owned(), 1)]),
    );
    report.by_origin =
        BTreeMap::from([("fixture_git".to_owned(), 6), ("product_git".to_owned(), 7)]);
    report.suite_total = 13;
    assert_eq!(
        check_budget(&report, &budget),
        Err("suite: 13 starts exceed 12".to_owned())
    );
    Ok(())
}

#[test]
fn expensive_test_growth_cannot_hide_below_the_suite_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    let mut report = measurement()?;
    report
        .tests
        .get_mut("board")
        .ok_or("missing board")?
        .insert("product_git".to_owned(), 5);
    report
        .tests
        .get_mut("gate")
        .ok_or("missing gate")?
        .insert("product_git".to_owned(), 0);
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("board: 6 starts exceed 5".to_owned())
    );
    Ok(())
}

#[test]
fn missing_setup_or_filtered_coverage_cannot_pass_the_suite_budget()
-> Result<(), Box<dyn std::error::Error>> {
    let mut report = measurement()?;
    report.setup.clear();
    report.suite_total = 8;
    report.by_origin =
        BTreeMap::from([("fixture_git".to_owned(), 3), ("product_git".to_owned(), 5)]);
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("missing SETUP process coverage".to_owned())
    );
    let mut report = measurement()?;
    report.suite_total -= 1;
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("suite, origin and test totals disagree".to_owned())
    );
    let mut report = measurement()?;
    report.tests.remove("gate");
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("incomplete full-suite test coverage".to_owned())
    );
    Ok(())
}

#[test]
fn missing_expensive_test_or_unresolved_attribution_cannot_pass_the_suite_budget()
-> Result<(), Box<dyn std::error::Error>> {
    let mut report = measurement()?;
    let board = report.tests.remove("board").ok_or("missing board")?;
    report.tests.insert("renamed".to_owned(), board);
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("missing expensive test: board".to_owned())
    );
    let mut report = measurement()?;
    report.unresolved_product = 1;
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("unresolved product starts".to_owned())
    );
    Ok(())
}

#[test]
fn origin_growth_cannot_hide_behind_a_constant_suite_total()
-> Result<(), Box<dyn std::error::Error>> {
    let mut report = measurement()?;
    report.tests.insert(
        "gate".to_owned(),
        BTreeMap::from([("fixture_git".to_owned(), 4), ("product_git".to_owned(), 0)]),
    );
    report.by_origin =
        BTreeMap::from([("fixture_git".to_owned(), 7), ("product_git".to_owned(), 4)]);
    assert_eq!(
        check_budget(&report, &budget()?),
        Err("fixture_git: 7 starts exceed 6".to_owned())
    );
    Ok(())
}
