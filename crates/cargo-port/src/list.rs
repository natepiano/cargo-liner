use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use walkdir::DirEntry;
use walkdir::WalkDir;

use super::output;
use super::project::RustProject;

#[derive(Args)]
pub struct ListArgs {
    /// Output as JSON instead of a table
    #[arg(long)]
    json: bool,

    /// Show workspace member crates (default: workspace roots only)
    #[arg(long)]
    members: bool,
}

/// Returns `true` if a directory entry should be visited during a walk.
/// Skips `target` (build artifacts) and `.git` (repository internals).
/// The `.git` directory is detected for project discovery but never
/// recursed into — its contents are irrelevant to scanning.
pub fn should_visit_entry(entry: &DirEntry) -> bool {
    if entry.file_type().is_dir() {
        let name = entry.file_name();
        return name != "target" && name != ".git";
    }
    true
}

pub fn scan_projects(scan_root: &Path) -> Vec<RustProject> {
    let mut projects = Vec::new();

    let entries = WalkDir::new(scan_root)
        .into_iter()
        .filter_entry(should_visit_entry);

    for entry in entries.flatten() {
        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            match RustProject::from_cargo_toml(entry.path()) {
                Ok(project) => projects.push(project),
                Err(e) => {
                    eprintln!("Warning: skipping {}: {e}", entry.path().display());
                },
            }
        }
    }

    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects
}

pub fn filter_workspace_members(projects: &mut Vec<RustProject>) {
    let workspace_paths: Vec<String> = projects
        .iter()
        .filter(|p| p.is_workspace())
        .map(|p| p.path.clone())
        .collect();

    projects.retain(|p| {
        if p.is_workspace() {
            return true;
        }
        !workspace_paths
            .iter()
            .any(|ws| p.path.starts_with(&format!("{ws}/")))
    });
}

#[allow(clippy::needless_pass_by_value)]
pub fn run(path: PathBuf, args: ListArgs) -> ExitCode {
    let Ok(scan_root) = path.canonicalize() else {
        eprintln!("Error: cannot resolve path '{}'", path.display());
        return ExitCode::FAILURE;
    };

    let mut projects = scan_projects(&scan_root);

    if !args.members {
        filter_workspace_members(&mut projects);
    }

    if args.json {
        output::render_json(&projects);
    } else {
        output::render_table(&projects);
    }

    ExitCode::SUCCESS
}
