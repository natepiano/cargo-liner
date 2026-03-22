use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use walkdir::WalkDir;

use crate::output;
use crate::project::RustProject;

#[derive(Args)]
pub struct ListArgs {
    /// Output as JSON instead of a table
    #[arg(long)]
    json: bool,

    /// Show workspace member crates (default: workspace roots only)
    #[arg(long)]
    members: bool,
}

pub fn scan_projects(scan_root: &Path) -> Vec<RustProject> {
    let mut projects = Vec::new();

    let entries = WalkDir::new(scan_root).into_iter().filter_entry(|entry| {
        if entry.file_type().is_dir() {
            let name = entry.file_name().to_string_lossy();
            !name.starts_with('.') && name != "target"
        } else {
            true
        }
    });

    for entry in entries.flatten() {
        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            match RustProject::from_cargo_toml(entry.path(), scan_root) {
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
        .map(|p| (*p.path).to_string())
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
