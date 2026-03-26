use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;

use walkdir::WalkDir;

use super::BackgroundMsg;
use super::FlatEntry;
use super::MemberGroup;
use super::ProjectNode;
use crate::ci::CiRun;
use crate::ci::GhRun;
use crate::ci::parse_owner_repo;
use crate::list::should_visit_entry;
use crate::project::GitInfo;
use crate::project::RustProject;

pub(super) const CACHE_DIR: &str = "cargo-port/ci-cache";

/// Base cache directory: `$TMPDIR/cargo-port/ci-cache`.
pub(super) fn cache_dir() -> Option<PathBuf> {
    std::env::var("TMPDIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/tmp")))
        .map(|d| d.join(CACHE_DIR))
}

/// Repo-keyed cache directory: `$TMPDIR/cargo-port/ci-cache/{owner}/{repo}`.
fn repo_cache_dir(owner: &str, repo: &str) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(owner).join(repo))
}

/// Public accessor for clearing the cache directory.
pub(super) fn repo_cache_dir_pub(owner: &str, repo: &str) -> Option<PathBuf> {
    repo_cache_dir(owner, repo)
}

const NO_MORE_RUNS_MARKER: &str = ".no_more_runs";

/// Check if the "no more runs" marker exists for a repo.
pub(super) fn is_exhausted(owner: &str, repo: &str) -> bool {
    repo_cache_dir(owner, repo).is_some_and(|d| d.join(NO_MORE_RUNS_MARKER).exists())
}

/// Save the "no more runs" marker for a repo.
pub(super) fn mark_exhausted(owner: &str, repo: &str) {
    if let Some(dir) = repo_cache_dir(owner, repo) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(NO_MORE_RUNS_MARKER), "");
    }
}

fn save_cached_run(owner: &str, repo: &str, ci_run: &CiRun) {
    let Some(dir) = repo_cache_dir(owner, repo) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", ci_run.run_id));
    if let Ok(json) = serde_json::to_string(ci_run) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_cached_run(owner: &str, repo: &str, run_id: u64) -> Option<CiRun> {
    let dir = repo_cache_dir(owner, repo)?;
    let path = dir.join(format!("{run_id}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Load all cached CI runs for a given repo.
pub(super) fn load_all_cached_runs(owner: &str, repo: &str) -> Vec<CiRun> {
    let Some(dir) = repo_cache_dir(owner, repo) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let contents = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str::<CiRun>(&contents).ok()
        })
        .collect()
}

/// Fetch recent CI runs from `gh run list`, then process each one (cache-first).
/// Returns the fetched/cached runs for the requested `count`.
fn fetch_recent_runs(
    repo_dir: &Path,
    repo_url: &str,
    owner: &str,
    repo: &str,
    gh_runs: &[GhRun],
) -> Vec<CiRun> {
    use crate::ci::process_gh_run;

    gh_runs
        .iter()
        .filter_map(|gh_run| {
            // Try cache first
            if let Some(cached) = load_cached_run(owner, repo, gh_run.database_id) {
                return Some(cached);
            }
            // Cache miss — fetch from `gh` and save
            let ci_run = process_gh_run(gh_run, repo_dir, repo_url)?;
            save_cached_run(owner, repo, &ci_run);
            Some(ci_run)
        })
        .collect()
}

/// Merge fetched + cached runs, deduplicated by `run_id`, sorted descending.
fn merge_runs(fetched: Vec<CiRun>, cached: Vec<CiRun>) -> Vec<CiRun> {
    let mut seen = HashSet::new();
    let mut merged: Vec<CiRun> = Vec::new();

    // Fetched runs take priority
    for run in fetched {
        if seen.insert(run.run_id) {
            merged.push(run);
        }
    }
    for run in cached {
        if seen.insert(run.run_id) {
            merged.push(run);
        }
    }

    merged.sort_by(|a, b| b.run_id.cmp(&a.run_id));
    merged
}

/// Fetch CI runs, using the repo-keyed cache. Merges freshly fetched runs
/// with all previously cached runs for this repo, deduplicated and sorted by `run_id` descending.
pub(super) fn fetch_ci_runs_cached(repo_dir: &Path, count: u32) -> Vec<CiRun> {
    use crate::ci::get_repo_url;
    use crate::ci::list_runs;

    let Some(repo_url) = get_repo_url(repo_dir) else {
        return Vec::new();
    };

    let Some((owner, repo)) = parse_owner_repo(&repo_url) else {
        return Vec::new();
    };

    let gh_runs = list_runs(repo_dir, None, count).unwrap_or_default();
    let fetched = fetch_recent_runs(repo_dir, &repo_url, &owner, &repo, &gh_runs);
    let cached = load_all_cached_runs(&owner, &repo);

    merge_runs(fetched, cached)
}

/// Fetch older CI runs beyond what we currently have, by requesting a larger
/// `--limit` from `gh run list` and returning any newly discovered runs.
pub(super) fn fetch_older_runs(repo_dir: &Path, current_count: u32) -> Vec<CiRun> {
    use crate::ci::get_repo_url;
    use crate::ci::list_runs;

    let Some(repo_url) = get_repo_url(repo_dir) else {
        return Vec::new();
    };

    let Some((owner, repo)) = parse_owner_repo(&repo_url) else {
        return Vec::new();
    };

    // Request 5 more runs than we currently have
    let fetch_count = current_count + 5;
    let gh_runs = list_runs(repo_dir, None, fetch_count).unwrap_or_default();
    let fetched = fetch_recent_runs(repo_dir, &repo_url, &owner, &repo, &gh_runs);

    // Only return the fetched runs — don't merge with the full cache.
    // The caller already has runs in memory; these get merged there.
    let mut result = fetched;
    result.sort_by(|a, b| b.run_id.cmp(&a.run_id));
    result
}

pub(super) fn fetch_crates_io_version(crate_name: &str) -> Option<String> {
    let url = format!("https://crates.io/api/v1/crates/{crate_name}");
    let output = std::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "5",
            "-H",
            "User-Agent: cargo-port",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json.get("crate")?
        .get("max_stable_version")?
        .as_str()
        .map(std::string::ToString::to_string)
}

pub(super) fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

pub(super) fn build_tree(projects: Vec<RustProject>, inline_dirs: &[String]) -> Vec<ProjectNode> {
    let workspace_paths: Vec<String> = projects
        .iter()
        .filter(|p| p.is_workspace())
        .map(|p| p.path.clone())
        .collect();

    let mut nodes: Vec<ProjectNode> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    let top_level_workspaces: HashSet<usize> = projects
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.is_workspace()
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != p.path && p.path.starts_with(&format!("{ws}/")))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, project) in projects.iter().enumerate() {
        if top_level_workspaces.contains(&i) {
            let mut all_members: Vec<RustProject> = projects
                .iter()
                .enumerate()
                .filter(|(j, p)| {
                    *j != i
                        && !top_level_workspaces.contains(j)
                        && p.path.starts_with(&format!("{}/", project.path))
                })
                .map(|(j, p)| {
                    consumed.insert(j);
                    p.clone()
                })
                .collect();

            all_members.sort_by(|a, b| {
                let name_a = a.name.as_deref().unwrap_or(&a.path);
                let name_b = b.name.as_deref().unwrap_or(&b.path);
                name_a.cmp(name_b)
            });

            let groups = group_members(&project.path, all_members, inline_dirs);

            consumed.insert(i);
            nodes.push(ProjectNode {
                project: project.clone(),
                groups,
                worktrees: Vec::new(),
                vendored: Vec::new(),
            });
        }
    }

    for (i, project) in projects.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        let under_workspace = workspace_paths
            .iter()
            .any(|ws| project.path.starts_with(&format!("{ws}/")));
        if !under_workspace {
            nodes.push(ProjectNode {
                project:   project.clone(),
                groups:    Vec::new(),
                worktrees: Vec::new(),
                vendored:  Vec::new(),
            });
        }
    }

    nodes.sort_by(|a, b| a.project.path.cmp(&b.project.path));

    // Detect vendored crates first, before worktree merging.
    // This catches crates like clay-layout that live inside worktree directories.
    extract_vendored(&mut nodes);

    // Merge worktree nodes into their primary project.
    // A worktree has `worktree_name = Some(...)`, the primary has `None`.
    merge_worktrees(&mut nodes);

    nodes
}

/// Group worktree nodes under their primary (non-worktree) project.
/// Projects match when they share the same package name.
/// The primary itself is also listed as a worktree entry (using its directory name).
fn merge_worktrees(nodes: &mut Vec<ProjectNode>) {
    let mut primary_indices: HashMap<String, usize> = HashMap::new();
    let mut worktree_indices: Vec<usize> = Vec::new();

    for (i, node) in nodes.iter().enumerate() {
        let Some(name) = &node.project.name else {
            continue;
        };
        if node.project.worktree_name.is_some() {
            worktree_indices.push(i);
        } else {
            primary_indices.insert(name.clone(), i);
        }
    }

    // Only process package names that actually have worktrees
    let names_with_worktrees: HashSet<String> = worktree_indices
        .iter()
        .filter_map(|&wi| nodes[wi].project.name.clone())
        .collect();

    // Collect worktree nodes to move (highest index first to preserve lower indices)
    let mut moves: Vec<(usize, String)> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            let name = nodes[wi].project.name.clone()?;
            primary_indices.get(&name)?;
            Some((wi, name))
        })
        .collect();
    moves.sort_by(|a, b| b.0.cmp(&a.0));

    let mut extracted: Vec<(ProjectNode, String)> = Vec::new();
    for (wi, name) in moves {
        let wt_node = nodes.remove(wi);
        extracted.push((wt_node, name));
    }

    // Insert worktree nodes into their primaries, and add the primary itself as a worktree entry
    for (wt_node, name) in extracted {
        if let Some(primary) = nodes.iter_mut().find(|n| {
            n.project.name.as_ref().is_some_and(|n| *n == name) && n.project.worktree_name.is_none()
        }) {
            primary.worktrees.push(wt_node);
        }
    }

    // Add the primary directory itself as the first worktree entry
    for name in &names_with_worktrees {
        if let Some(primary) = nodes.iter_mut().find(|n| {
            n.project.name.as_ref().is_some_and(|n| n == name) && n.project.worktree_name.is_none()
        }) {
            let dir_name = primary
                .project
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&primary.project.path)
                .to_string();
            let mut primary_as_wt = primary.project.clone();
            primary_as_wt.worktree_name = Some(dir_name);
            primary.worktrees.insert(
                0,
                ProjectNode {
                    project:   primary_as_wt,
                    groups:    Vec::new(),
                    worktrees: Vec::new(),
                    vendored:  Vec::new(),
                },
            );
        }
    }
}

/// Find standalone nodes whose path lives inside another node's directory
/// (or inside a worktree's directory) and move them into that node's `vendored` list.
fn extract_vendored(nodes: &mut Vec<ProjectNode>) {
    // Collect abs_paths of all nodes and their worktrees
    let mut parent_paths: Vec<(usize, Option<usize>, String)> = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        parent_paths.push((ni, None, node.project.abs_path.clone()));
        for (wi, wt) in node.worktrees.iter().enumerate() {
            parent_paths.push((ni, Some(wi), wt.project.abs_path.clone()));
        }
    }

    // Find which top-level nodes are vendored inside another
    let mut vendored_map: Vec<(usize, usize, Option<usize>)> = Vec::new(); // (vendored_node_idx, parent_node_idx, parent_wt_idx)

    for (vi, vnode) in nodes.iter().enumerate() {
        // Skip nodes that have workspace members or worktrees — they're real projects
        if vnode.has_members() || !vnode.worktrees.is_empty() {
            continue;
        }
        for &(ni, wt_idx, ref parent_abs) in &parent_paths {
            if ni == vi {
                continue;
            }
            if vnode
                .project
                .abs_path
                .starts_with(&format!("{parent_abs}/"))
            {
                vendored_map.push((vi, ni, wt_idx));
                break;
            }
        }
    }

    if vendored_map.is_empty() {
        return;
    }

    // Extract vendored projects (iterate in reverse to preserve indices)
    let mut vendored_projects: Vec<(usize, Option<usize>, RustProject)> = Vec::new();
    let mut remove_indices: Vec<usize> = vendored_map.iter().map(|&(vi, _, _)| vi).collect();
    remove_indices.sort_unstable();
    remove_indices.dedup();

    for &(vi, ni, wt_idx) in &vendored_map {
        vendored_projects.push((ni, wt_idx, nodes[vi].project.clone()));
    }

    // Remove vendored nodes from the top level (reverse order)
    for &idx in remove_indices.iter().rev() {
        nodes.remove(idx);
    }

    // Adjust parent indices after removal
    for (ni, wt_idx, project) in vendored_projects {
        let adjusted_ni = remove_indices.iter().filter(|&&r| r < ni).count();
        let target_ni = ni - adjusted_ni;
        if let Some(node) = nodes.get_mut(target_ni) {
            if let Some(wi) = wt_idx {
                if let Some(wt) = node.worktrees.get_mut(wi) {
                    wt.vendored.push(project);
                }
            } else {
                node.vendored.push(project);
            }
        }
    }

    // Sort vendored lists
    for node in nodes {
        node.vendored.sort_by(|a, b| a.path.cmp(&b.path));
        for wt in &mut node.worktrees {
            wt.vendored.sort_by(|a, b| a.path.cmp(&b.path));
        }
    }
}

pub(super) fn group_members(
    workspace_path: &str,
    members: Vec<RustProject>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let prefix = format!("{workspace_path}/");

    let mut group_map: HashMap<String, Vec<RustProject>> = HashMap::new();

    for member in members {
        let relative = member.path.strip_prefix(&prefix).unwrap_or(&member.path);
        let subdir = relative.split('/').next().unwrap_or("").to_string();

        // Members in configured inline dirs or directly in the workspace root are shown inline.
        // Everything else gets grouped by first subdirectory.
        let group_name = if inline_dirs.contains(&subdir) || !relative.contains('/') {
            String::new()
        } else {
            subdir
        };

        group_map.entry(group_name).or_default().push(member);
    }

    let mut groups: Vec<MemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| MemberGroup { name, members })
        .collect();

    // Sort: named directories first (alphabetically), then inline group last
    groups.sort_by(|a, b| {
        let a_inline = a.name.is_empty();
        let b_inline = b.name.is_empty();
        match (a_inline, b_inline) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.name.cmp(&b.name),
        }
    });

    groups
}

pub(super) fn build_flat_entries(nodes: &[ProjectNode]) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        // Add workspace root itself
        let name = node.project.name.as_deref().unwrap_or(&node.project.path);
        entries.push(FlatEntry {
            node_index:   ni,
            group_index:  0,
            member_index: 0,
            name:         name.to_string(),
        });
        // Add all members
        for (gi, group) in node.groups.iter().enumerate() {
            for (mi, member) in group.members.iter().enumerate() {
                let name = member.name.as_deref().unwrap_or(&member.path);
                entries.push(FlatEntry {
                    node_index:   ni,
                    group_index:  gi,
                    member_index: mi,
                    name:         name.to_string(),
                });
            }
        }
    }
    entries
}

/// Fetch all details (disk, git, crates.io, CI) for a single project and send
/// results through the provided channel. Used by both the main scan and priority fetch.
pub(super) fn fetch_project_details(
    tx: &mpsc::Sender<BackgroundMsg>,
    project_path: &str,
    abs_path: &Path,
    project_name: Option<&String>,
    has_git: bool,
    ci_run_count: u32,
) {
    // Git info first (local, instant)
    if has_git && let Some(info) = GitInfo::detect(abs_path) {
        let _ = tx.send(BackgroundMsg::GitInfo {
            path: project_path.to_string(),
            info,
        });
    }

    // Disk usage (local but slow for large projects)
    let bytes = dir_size(abs_path);
    let _ = tx.send(BackgroundMsg::DiskUsage {
        path: project_path.to_string(),
        bytes,
    });

    // CI runs (network, can be slow)
    if has_git {
        let _ = tx.send(BackgroundMsg::ScanActivity {
            path: format!("CI: {project_path}"),
        });
        let runs = fetch_ci_runs_cached(abs_path, ci_run_count);
        let _ = tx.send(BackgroundMsg::CiRuns {
            path: project_path.to_string(),
            runs,
        });
    }

    // Crates.io version (network)
    if let Some(name) = project_name
        && let Some(version) = fetch_crates_io_version(name)
    {
        let _ = tx.send(BackgroundMsg::CratesIoVersion {
            path: project_path.to_string(),
            version,
        });
    }

    // GitHub stars (network)
    if has_git
        && let Some(repo_url) = crate::ci::get_repo_url(abs_path)
        && let Some((owner, repo)) = crate::ci::parse_owner_repo(&repo_url)
        && let Some(count) = fetch_star_count(&owner, &repo)
    {
        let _ = tx.send(BackgroundMsg::Stars {
            path: project_path.to_string(),
            count,
        });
    }
}

/// Fetch the star count for a GitHub repo.
fn fetch_star_count(owner: &str, repo: &str) -> Option<u64> {
    let output = std::process::Command::new("gh")
        .args([
            "api",
            &format!("repos/{owner}/{repo}"),
            "--jq",
            ".stargazers_count",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Spawn a streaming scan: walk the directory tree, and for each project discovered
/// do disk + CI together on rayon so progress fills in visibly.
/// Returns `(Sender, Receiver)` — the sender is retained by the caller for priority fetches.
///
/// When `include_non_rust` is true, directories containing `.git` (directory, not file)
/// but no `Cargo.toml` are also discovered as non-Rust projects.
pub(super) fn spawn_streaming_scan(
    scan_root: &Path,
    ci_run_count: u32,
    exclude_dirs: &[String],
    include_non_rust: bool,
) -> (mpsc::Sender<BackgroundMsg>, Receiver<BackgroundMsg>) {
    let (tx, rx) = mpsc::channel();
    let root = scan_root.to_path_buf();
    let excludes: HashSet<String> = exclude_dirs.iter().cloned().collect();

    let scan_tx = tx.clone();
    thread::spawn(move || {
        let entries = WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| should_visit_entry(entry, Some(&excludes)));

        rayon::scope(|s| {
            for entry in entries.flatten() {
                if entry.file_type().is_dir() {
                    let rel = entry
                        .path()
                        .strip_prefix(&root)
                        .unwrap_or_else(|_| entry.path())
                        .display()
                        .to_string();
                    let _ = scan_tx.send(BackgroundMsg::ScanActivity {
                        path: if rel.is_empty() { ".".to_string() } else { rel },
                    });

                    // Non-Rust project detection: .git dir present but no Cargo.toml
                    if include_non_rust
                        && entry.path().join(".git").is_dir()
                        && !entry.path().join("Cargo.toml").exists()
                    {
                        let project = RustProject::from_git_dir(entry.path());
                        let abs_path = PathBuf::from(&project.abs_path);

                        let _ = scan_tx.send(BackgroundMsg::ProjectDiscovered {
                            project: project.clone(),
                        });

                        let task_tx = scan_tx.clone();
                        let task_path = project.path.clone();
                        let task_abs = abs_path;
                        s.spawn(move |_| {
                            fetch_project_details(
                                &task_tx,
                                &task_path,
                                &task_abs,
                                None,
                                true,
                                ci_run_count,
                            );
                        });
                    }
                }
                if entry.file_type().is_file()
                    && entry.file_name() == "Cargo.toml"
                    && let Ok(project) = RustProject::from_cargo_toml(entry.path())
                {
                    let abs_path = PathBuf::from(&project.abs_path);
                    let has_git = abs_path.join(".git").exists();

                    let _ = scan_tx.send(BackgroundMsg::ProjectDiscovered {
                        project: project.clone(),
                    });

                    // Spawn one rayon task per project that does disk + CI together
                    let task_tx = scan_tx.clone();
                    let task_path = project.path.clone();
                    let task_name = project.name.clone();
                    let task_abs = abs_path;
                    s.spawn(move |_| {
                        fetch_project_details(
                            &task_tx,
                            &task_path,
                            &task_abs,
                            task_name.as_ref(),
                            has_git,
                            ci_run_count,
                        );
                    });
                }
            }
        });

        let _ = scan_tx.send(BackgroundMsg::ScanComplete);
    });

    (tx, rx)
}
