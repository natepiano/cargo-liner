//! Testing utility used to measure tracked-row path discovery.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use walkdir::WalkDir;

#[derive(Parser)]
struct Args {
    /// Root directory used for testing, e.g. ~/rust
    root: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProjectEntry {
    abs_path: PathBuf,
    path: String,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    let root = expand_home(&args.root)?;
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }

    let started = Instant::now();
    let mut projects = discover_projects(&root);
    projects.sort();
    let tracked = collect_tracked_row_paths(&projects);
    let node_count = top_level_project_count(&projects);
    let elapsed = started.elapsed();

    println!("root: {}", root.display());
    println!("project_count: {}", projects.len());
    println!("node_count: {node_count}");
    println!("tracked_row_count: {}", tracked.len());
    println!("elapsed_ms: {}", elapsed.as_millis());
    println!();
    for path in tracked {
        println!("{path}");
    }

    Ok(())
}

fn expand_home(raw: &str) -> Result<PathBuf, String> {
    if raw == "~" {
        return dirs::home_dir().ok_or_else(|| "home directory not found".to_string());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|home| home.join(rest))
            .ok_or_else(|| "home directory not found".to_string());
    }
    Ok(PathBuf::from(raw))
}

fn discover_projects(root: &Path) -> Vec<ProjectEntry> {
    let mut projects = Vec::new();
    let mut iter = WalkDir::new(root).into_iter();

    while let Some(Ok(entry)) = iter.next() {
        if entry.file_type().is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                iter.skip_current_dir();
                continue;
            }
        }

        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            let Some(project_dir) = entry.path().parent() else {
                continue;
            };
            projects.push(ProjectEntry {
                abs_path: project_dir.to_path_buf(),
                path: tilde_path(project_dir),
            });
        }
    }

    projects
}

fn tilde_path(path: &Path) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.display().to_string();
    };
    path.strip_prefix(&home).map_or_else(
        |_| path.display().to_string(),
        |rest| {
            if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            }
        },
    )
}

fn collect_tracked_row_paths(projects: &[ProjectEntry]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    for project in projects {
        if seen.insert(project.path.clone()) {
            paths.push(project.path.clone());
        }
    }

    paths
}

fn top_level_project_count(projects: &[ProjectEntry]) -> usize {
    let mut count = 0usize;
    for project in projects {
        let is_nested = projects.iter().any(|candidate| {
            candidate.abs_path != project.abs_path
                && project.abs_path.starts_with(&candidate.abs_path)
        });
        if !is_nested {
            count += 1;
        }
    }
    count
}
