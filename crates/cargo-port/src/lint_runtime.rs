use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use notify::RecursiveMode;
use notify::Watcher;

use crate::cache_paths;
use crate::config::Config;
use crate::config::LintCommandConfig;
use crate::config::LintConfig;
use crate::port_report;

const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
const IDLE_POLL: Duration = Duration::from_secs(3600);
const LEGACY_LINT_WATCHER_PATH: &str = ".claude/scripts/lint-watcher/";

pub struct RegisterProjectRequest {
    pub project_path: String,
    pub abs_path:     PathBuf,
    pub is_rust:      bool,
}

pub struct SpawnResult {
    pub register_tx: Option<mpsc::Sender<RegisterProjectRequest>>,
    pub warning:     Option<String>,
}

pub fn spawn(config: &Config) -> SpawnResult {
    if !config.lint.enabled {
        return SpawnResult {
            register_tx: None,
            warning: None,
        };
    }

    if legacy_watcher_active() {
        return SpawnResult {
            register_tx: None,
            warning: Some(
                "external lint watcher detected; internal lint runtime disabled".to_string(),
            ),
        };
    }

    let cache_root = cache_paths::port_report_root_for(config);
    let lint = config.lint.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || supervisor_loop(rx, cache_root, lint));
    SpawnResult {
        register_tx: Some(tx),
        warning: None,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "supervisor owns its channel and config for the lifetime of the thread"
)]
fn supervisor_loop(
    register_rx: mpsc::Receiver<RegisterProjectRequest>,
    cache_root: PathBuf,
    lint: LintConfig,
) {
    let mut watched = HashSet::new();
    let commands = lint.resolved_commands();
    while let Ok(request) = register_rx.recv() {
        if !should_watch_project(&lint, &request) {
            continue;
        }
        if !watched.insert(request.abs_path.clone()) {
            continue;
        }
        spawn_project_worker(request.abs_path, cache_root.clone(), commands.clone());
    }
}

fn spawn_project_worker(project_root: PathBuf, cache_root: PathBuf, commands: Vec<LintCommandConfig>) {
    thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        let handler = move |res| {
            let _ = event_tx.send(res);
        };
        let Ok(mut watcher) = notify::recommended_watcher(handler) else {
            return;
        };
        if watcher.watch(&project_root, RecursiveMode::Recursive).is_err() {
            return;
        }

        let mut next_run_at = None;
        loop {
            let timeout = next_run_at.map_or(IDLE_POLL, |deadline: Instant| {
                deadline.saturating_duration_since(Instant::now())
            });
            match event_rx.recv_timeout(timeout) {
                Ok(Ok(event)) => {
                    if event.paths.iter().any(|path| is_relevant_change(&project_root, path)) {
                        next_run_at = Some(Instant::now() + LINT_DEBOUNCE);
                    }
                },
                Ok(Err(_)) => {},
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if next_run_at.take().is_some() {
                        let _ = run_commands_for_project(&project_root, &cache_root, &commands);
                    }
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn should_watch_project(lint: &LintConfig, request: &RegisterProjectRequest) -> bool {
    if !request.is_rust || !request.abs_path.join("Cargo.toml").is_file() {
        return false;
    }
    if !matches_prefixes(&lint.include, &request.project_path, &request.abs_path, true) {
        return false;
    }
    !matches_prefixes(&lint.exclude, &request.project_path, &request.abs_path, false)
}

fn matches_prefixes(
    prefixes: &[String],
    project_path: &str,
    abs_path: &Path,
    empty_means_match: bool,
) -> bool {
    if prefixes.is_empty() {
        return empty_means_match;
    }
    let abs = abs_path.to_string_lossy();
    prefixes
        .iter()
        .any(|prefix| project_path.starts_with(prefix) || abs.starts_with(prefix))
}

fn is_relevant_change(project_root: &Path, path: &Path) -> bool {
    if !path.starts_with(project_root) {
        return false;
    }
    if path.components().any(|component| {
        let part = component.as_os_str();
        part == "target" || part == ".git"
    }) {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name == "Cargo.toml"
        || file_name == "Cargo.lock"
        || path.extension().is_some_and(|ext| ext == "rs")
}

pub fn run_commands_for_project(
    project_root: &Path,
    cache_root: &Path,
    commands: &[LintCommandConfig],
) -> io::Result<()> {
    let output_dir = port_report::output_dir_under(cache_root, project_root);
    std::fs::create_dir_all(&output_dir)?;
    port_report::append_status_under(cache_root, project_root, "started")?;

    let manifest_path = project_root.join("Cargo.toml");
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !run_command(project_root, &manifest_path, &output_dir, command, index)? {
            failed = true;
        }
    }

    port_report::append_status_under(
        cache_root,
        project_root,
        if failed { "failed" } else { "passed" },
    )?;
    Ok(())
}

fn run_command(
    project_root: &Path,
    manifest_path: &Path,
    output_dir: &Path,
    command: &LintCommandConfig,
    index: usize,
) -> io::Result<bool> {
    let log_name = command_log_name(command, index);
    let log_path = output_dir.join(format!("{log_name}-latest.log"));
    let tmp_path = output_dir.join(format!("{log_name}-latest.log.tmp"));

    let shell_output = Command::new("/bin/sh")
        .arg("-lc")
        .arg(&command.command)
        .current_dir(project_root)
        .env("PROJECT_DIR", project_root)
        .env("MANIFEST_PATH", manifest_path)
        .env("PORT_REPORT_DIR", output_dir)
        .output();

    let (success, bytes) = match shell_output {
        Ok(output) => {
            let mut bytes = output.stdout;
            bytes.extend_from_slice(&output.stderr);
            (output.status.success(), bytes)
        },
        Err(err) => (
            false,
            format!("failed to spawn lint command '{}': {err}\n", command.command).into_bytes(),
        ),
    };

    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(tmp_path, log_path)?;
    Ok(success)
}

fn command_log_name(command: &LintCommandConfig, index: usize) -> String {
    let base = if command.name.trim().is_empty() {
        format!("command-{}", index + 1)
    } else {
        command.name.trim().to_string()
    };
    let sanitized = sanitize_name(&base);
    if sanitized.is_empty() {
        format!("command-{}", index + 1)
    } else {
        sanitized
    }
}

fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').to_string()
}

fn legacy_watcher_active() -> bool {
    let Ok(output) = Command::new("ps").args(["ax", "-o", "command="]).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8(output.stdout)
        .ok()
        .is_some_and(|stdout| contains_legacy_watcher(&stdout))
}

fn contains_legacy_watcher(ps_output: &str) -> bool {
    ps_output
        .lines()
        .any(|line| line.contains(LEGACY_LINT_WATCHER_PATH))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn request(path: &str, abs_path: &Path, is_rust: bool) -> RegisterProjectRequest {
        RegisterProjectRequest {
            project_path: path.to_string(),
            abs_path: abs_path.to_path_buf(),
            is_rust,
        }
    }

    #[test]
    fn include_and_exclude_filters_match_display_or_absolute_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(project_dir.path().join("Cargo.toml"), "[package]\nname='demo'\nversion='0.1.0'\n")
            .expect("write manifest");
        let lint = LintConfig {
            enabled: true,
            include: vec!["~/rust/demo".to_string()],
            exclude: vec![project_dir.path().to_string_lossy().to_string()],
            commands: Vec::new(),
        };

        let req = request("~/rust/demo", project_dir.path(), true);
        assert!(!should_watch_project(&lint, &req));

        let lint = LintConfig {
            exclude: Vec::new(),
            ..lint
        };
        assert!(should_watch_project(&lint, &req));
    }

    #[test]
    fn non_rust_projects_are_never_watched() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let req = request("~/rust/not-rust", project_dir.path(), false);
        assert!(!should_watch_project(&LintConfig::default(), &req));
    }

    #[test]
    fn relevant_changes_ignore_git_and_target_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        assert!(is_relevant_change(project_dir.path(), &project_dir.path().join("src/main.rs")));
        assert!(is_relevant_change(project_dir.path(), &project_dir.path().join("Cargo.toml")));
        assert!(!is_relevant_change(project_dir.path(), &project_dir.path().join("target/debug/app")));
        assert!(!is_relevant_change(project_dir.path(), &project_dir.path().join(".git/index")));
    }

    #[test]
    fn detects_legacy_watcher_from_ps_output() {
        let ps = "/bin/bash /Users/natemccoy/.claude/scripts/lint-watcher/lint-watcher.sh\n";
        assert!(contains_legacy_watcher(ps));
        assert!(!contains_legacy_watcher("cargo-port .\n"));
    }

    #[test]
    fn lint_commands_write_reports_under_configured_cache_root() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");

        let mut cfg = Config::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        let cache_root = cache_paths::port_report_root_for(&cfg);
        let commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "printf 'lint ok\\n'".to_string(),
        }];

        run_commands_for_project(project_dir.path(), &cache_root, &commands).expect("run commands");

        let log_path = port_report::log_path_under(&cache_root, project_dir.path());
        let report_dir = port_report::output_dir_under(&cache_root, project_dir.path());
        let protocol = std::fs::read_to_string(log_path).expect("read protocol log");
        let report = std::fs::read_to_string(report_dir.join("echo-latest.log"))
            .expect("read command report");

        assert!(protocol.contains("\tstarted\n"));
        assert!(protocol.contains("\tpassed\n"));
        assert_eq!(report, "lint ok\n");
    }
}
