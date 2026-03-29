use colored::Colorize;
use comfy_table::CellAlignment;
use comfy_table::Table;
use comfy_table::presets::ASCII_MARKDOWN;

use crate::ci;
use crate::ci::CiRun;
use crate::project::RustProject;

pub fn render_table(projects: &[RustProject]) {
    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(bold_headers(&["Path", "Name", "Version", "Types"]));

    for project in projects {
        let name = project.name.as_deref().unwrap_or("-");
        let version = project.version.as_deref().unwrap_or("-");
        let types = project
            .types
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        table.add_row(vec![&project.path, name, version, &types]);
    }

    println!("{table}");
}

pub fn render_json(projects: &[RustProject]) {
    match serde_json::to_string_pretty(projects) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("Failed to serialize projects: {e}"),
    }
}

pub fn render_ci_table(runs: &[CiRun]) {
    if runs.len() == 1 {
        render_ci_single(&runs[0]);
    } else {
        render_ci_multi(runs);
    }
}

fn render_ci_single(ci_run: &CiRun) {
    println!(
        "=== Run {} ({}) [{}] ===",
        ci_run.run_id, ci_run.created_at, ci_run.branch
    );

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(bold_headers(&["Job", "Result", "Duration"]));
    right_align_column(&mut table, 2);

    for job in &ci_run.jobs {
        let result = colorize_conclusion(&job.conclusion);
        table.add_row(vec![&job.name, &result, &job.duration]);
    }

    if let Some(secs) = ci_run.wall_clock_secs {
        let result = colorize_conclusion(&ci_run.conclusion);
        table.add_row(vec!["Total".to_string(), result, ci::format_secs(secs)]);
    }

    println!("{table}");
    println!("{}", ci_run.url);
}

fn render_ci_multi(runs: &[CiRun]) {
    // Collect all unique job names in order of first appearance
    let mut job_names: Vec<String> = Vec::new();
    for ci_run in runs {
        for job in &ci_run.jobs {
            if !job_names.contains(&job.name) {
                job_names.push(job.name.clone());
            }
        }
    }

    // Short run labels: just the index (1, 2, 3...) — the reference table maps them
    let run_labels: Vec<String> = (1..=runs.len()).map(|i| format!("{i}")).collect();

    render_per_job_tables(runs, &job_names, &run_labels);
    render_total_time_table(runs, &run_labels);
    render_runs_reference_table(runs, &run_labels);
}

/// Renders a table per CI job showing results across all runs.
fn render_per_job_tables(runs: &[CiRun], job_names: &[String], run_labels: &[String]) {
    for job_name in job_names {
        println!("{}", job_name.bold().yellow());

        let longest_job_idx = runs
            .iter()
            .enumerate()
            .max_by_key(|(_, r)| {
                r.jobs
                    .iter()
                    .find(|j| j.name == *job_name)
                    .and_then(|j| j.duration_secs)
                    .unwrap_or(0)
            })
            .map(|(i, _)| i);

        let mut table = Table::new();
        table.load_preset(ASCII_MARKDOWN);
        table.set_header(bold_headers(&[
            "Run", "Branch", "Date", "Time", "Result", "Duration",
        ]));
        right_align_column(&mut table, 5);

        for (i, ci_run) in runs.iter().enumerate() {
            let (date, time) = format_datetime(&ci_run.created_at);
            let is_longest = longest_job_idx == Some(i);
            let job = ci_run.jobs.iter().find(|j| j.name == *job_name);
            if let Some(j) = job {
                let result = format_result(&j.conclusion, is_longest);
                table.add_row(vec![
                    &run_labels[i],
                    &ci_run.branch,
                    &date,
                    &time,
                    &result,
                    &j.duration,
                ]);
            } else {
                let result = format_result("—", is_longest);
                table.add_row(vec![
                    &run_labels[i] as &str,
                    &ci_run.branch,
                    &date,
                    &time,
                    &result,
                    "—",
                ]);
            }
        }

        println!("{table}");
        println!();
    }
}

/// Renders the total wall-clock time table across all runs.
fn render_total_time_table(runs: &[CiRun], run_labels: &[String]) {
    println!(
        "{}",
        "Total (latest completion minus earliest start)"
            .bold()
            .yellow()
    );

    let longest_idx = runs
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| r.wall_clock_secs.unwrap_or(0))
        .map(|(i, _)| i);

    let mut total_table = Table::new();
    total_table.load_preset(ASCII_MARKDOWN);
    total_table.set_header(bold_headers(&[
        "Run", "Branch", "Date", "Time", "Result", "Duration",
    ]));
    right_align_column(&mut total_table, 5);

    for (i, ci_run) in runs.iter().enumerate() {
        let (date, time) = format_datetime(&ci_run.created_at);
        let duration = ci_run
            .wall_clock_secs
            .map_or_else(|| "—".to_string(), ci::format_secs);
        let result = format_result(&ci_run.conclusion, longest_idx == Some(i));
        total_table.add_row(vec![
            &run_labels[i],
            &ci_run.branch,
            &date,
            &time,
            &result,
            &duration,
        ]);
    }

    println!("{total_table}");
    println!("* = longest run");
    println!();
}

/// Renders a reference table mapping run numbers to branches, dates, and URLs.
fn render_runs_reference_table(runs: &[CiRun], run_labels: &[String]) {
    println!("{}", "Runs".bold().yellow());

    let mut ref_table = Table::new();
    ref_table.load_preset(ASCII_MARKDOWN);
    ref_table.set_header(bold_headers(&[
        "Run", "Branch", "Date", "Time", "Result", "URL",
    ]));

    for (i, ci_run) in runs.iter().enumerate() {
        let (date, time) = format_datetime(&ci_run.created_at);
        let result = colorize_conclusion(&ci_run.conclusion);
        ref_table.add_row(vec![
            &run_labels[i],
            &ci_run.branch,
            &date,
            &time,
            &result,
            &ci_run.url,
        ]);
    }

    println!("{ref_table}");
}

fn colorize_conclusion(conclusion: &str) -> String {
    let padded = format!("  {conclusion}");
    if padded.contains('✓') {
        padded.green().to_string()
    } else if padded.contains('✗') {
        padded.red().to_string()
    } else {
        padded
    }
}

fn format_result(conclusion: &str, is_longest: bool) -> String {
    let suffix = if is_longest { " *" } else { "" };
    let label = format!("  {conclusion}{suffix}");
    if label.contains('✓') {
        label.green().to_string()
    } else if label.contains('✗') {
        label.red().to_string()
    } else {
        label
    }
}

fn right_align_column(table: &mut Table, col: usize) {
    if let Some(column) = table.column_mut(col) {
        column.set_cell_alignment(CellAlignment::Right);
    }
}

fn bold_headers(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|l| l.bold().to_string()).collect()
}

/// Splits an ISO 8601 timestamp into `(YYYY-MM-DD, HH:MM:SS)`.
fn format_datetime(iso: &str) -> (String, String) {
    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => ((*date).to_string(), (*time).to_string()),
        None => ((*stripped).to_string(), "—".to_string()),
    }
}

pub fn render_ci_json(runs: &[CiRun]) {
    match serde_json::to_string_pretty(runs) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("Failed to serialize CI runs: {e}"),
    }
}
