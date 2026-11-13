use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use ignore::Match;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;

use super::command;
use super::command::GitAnswer;
use super::constants::GIT_CACHED_ARG;
use super::constants::GIT_CONFIG_COMMAND;
use super::constants::GIT_CORE_EXCLUDESFILE_KEY;
use super::constants::GIT_CORE_IGNORECASE_KEY;
use super::constants::GIT_GET_ARG;
use super::constants::GIT_LIST_ARG;
use super::constants::GIT_LS_FILES_COMMAND;
use super::constants::GIT_NULL_ARG;
use super::constants::GIT_PATH_ARG;
use super::constants::GIT_SHOW_ORIGIN_ARG;
use super::constants::GIT_TYPE_BOOL_ARG;
use super::discovery;
use crate::project::AbsolutePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitEventRefresh {
    Required,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitPathCase {
    Sensitive,
    Insensitive,
    Unresolved,
}

#[derive(Debug)]
enum TrackedGitPaths {
    Known(HashSet<PathBuf>),
    Unreadable,
}

#[derive(Debug)]
enum GitIgnoreRules {
    Known(Gitignore),
    Unreadable,
}

#[derive(Debug)]
enum GlobalIgnoreFile {
    Configured(PathBuf),
    Unconfigured,
}

#[derive(Debug, PartialEq, Eq)]
enum RuleSourceStamp {
    Missing,
    Present { length: u64, modified: SystemTime },
    Unreadable,
}

impl RuleSourceStamp {
    fn read(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(metadata) => {
                metadata
                    .modified()
                    .map_or(Self::Unreadable, |modified| Self::Present {
                        length: metadata.len(),
                        modified,
                    })
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Unreadable,
        }
    }
}

#[derive(Debug)]
struct CheckoutEventVisibility {
    root:         AbsolutePath,
    tracked:      TrackedGitPaths,
    global:       GitIgnoreRules,
    exclude:      GitIgnoreRules,
    directories:  HashMap<PathBuf, GitIgnoreRules>,
    path_case:    GitPathCase,
    rule_sources: HashMap<PathBuf, RuleSourceStamp>,
}

/// `GitEventVisibilityCache` belongs to one watcher entry. The UI never locks this cache.
#[derive(Debug, Default)]
pub(crate) struct GitEventVisibilityCache {
    checkouts:    HashMap<AbsolutePath, CheckoutEventVisibility>,
    rule_sources: HashSet<PathBuf>,
}

impl GitEventVisibilityCache {
    pub(crate) fn invalidate(&mut self, path: &Path) {
        self.checkouts.retain(|root, checkout| {
            checkout
                .directories
                .retain(|directory, _| !directory.starts_with(path));
            !(checkout.rule_sources.contains_key(path)
                || path.starts_with(root)
                    && path.file_name().is_some_and(|name| name == ".gitignore")
                || path == root.join(".git/index"))
        });
    }

    pub(crate) fn is_rule_source(
        &self,
        root: &AbsolutePath,
        git_dir: &AbsolutePath,
        common_git_dir: &AbsolutePath,
        path: &Path,
    ) -> bool {
        path.starts_with(root) && path.file_name().is_some_and(|name| name == ".gitignore")
            || path == common_git_dir.join("info/exclude")
            || path == common_git_dir.join("config")
            || path == git_dir.join("config.worktree")
            || default_config_paths().iter().any(|source| source == path)
            || self.rule_sources.contains(path)
    }

    pub(crate) fn requires_refresh(
        &mut self,
        root: &AbsolutePath,
        git_dir: &AbsolutePath,
        path: &Path,
    ) -> GitEventRefresh {
        if self
            .checkouts
            .get(root)
            .is_some_and(|checkout| !checkout.sources_are_current())
        {
            self.checkouts.remove(root);
        }
        let checkout = self
            .checkouts
            .entry(root.clone())
            .or_insert_with(|| CheckoutEventVisibility::load(root, git_dir));
        self.rule_sources
            .extend(checkout.rule_sources.keys().cloned());
        checkout.requires_refresh(path)
    }
}

impl CheckoutEventVisibility {
    fn load(root: &AbsolutePath, git_dir: &AbsolutePath) -> Self {
        let path_case = match command::git_answer_logged(
            root,
            "watcher_ignore_case",
            &[
                GIT_CONFIG_COMMAND,
                GIT_TYPE_BOOL_ARG,
                GIT_GET_ARG,
                GIT_CORE_IGNORECASE_KEY,
            ],
        ) {
            Ok(GitAnswer::Yes(output)) if output.trimmed() == "true" => GitPathCase::Insensitive,
            Ok(GitAnswer::No) => GitPathCase::Sensitive,
            Ok(GitAnswer::Yes(output)) if output.trimmed() == "false" => GitPathCase::Sensitive,
            _ => GitPathCase::Unresolved,
        };
        let tracked = command::git_output_logged(
            root,
            "watcher_tracked_paths",
            &[GIT_LS_FILES_COMMAND, GIT_NULL_ARG, GIT_CACHED_ARG],
        )
        .map_or(TrackedGitPaths::Unreadable, |output| {
            TrackedGitPaths::Known(
                output
                    .nul_paths()
                    .flat_map(|path| {
                        path.ancestors()
                            .take_while(|ancestor| !ancestor.as_os_str().is_empty())
                            .map(|ancestor| tracked_path_key(ancestor, path_case))
                    })
                    .collect(),
            )
        });
        let common_git_dir =
            discovery::resolve_common_git_dir(root).unwrap_or_else(|| git_dir.clone());
        let global_file = match command::git_answer_logged(
            root,
            "watcher_global_ignore",
            &[
                GIT_CONFIG_COMMAND,
                GIT_PATH_ARG,
                GIT_GET_ARG,
                GIT_CORE_EXCLUDESFILE_KEY,
            ],
        ) {
            Ok(GitAnswer::Yes(output)) => GlobalIgnoreFile::Configured(root.join(output.trimmed())),
            Ok(GitAnswer::No) => ignore::gitignore::gitconfig_excludes_path()
                .map_or(GlobalIgnoreFile::Unconfigured, GlobalIgnoreFile::Configured),
            Err(_) => GlobalIgnoreFile::Unconfigured,
        };
        let exclude_path = common_git_dir.join("info/exclude");
        let mut sources = default_config_paths();
        sources.extend([
            common_git_dir.join("config"),
            git_dir.join("config.worktree"),
            git_dir.join("index"),
            exclude_path.clone(),
        ]);
        let mut global = match &global_file {
            GlobalIgnoreFile::Configured(file) => {
                sources.push(file.clone());
                GitIgnoreRules::load(root, file, path_case)
            },
            GlobalIgnoreFile::Unconfigured => GitIgnoreRules::Known(Gitignore::empty()),
        };
        include_config_sources(root, &mut sources, &mut global);
        Self {
            root: root.clone(),
            tracked,
            global,
            exclude: GitIgnoreRules::load(root, &exclude_path, path_case),
            directories: HashMap::new(),
            path_case,
            rule_sources: sources
                .into_iter()
                .map(|path| {
                    let stamp = RuleSourceStamp::read(&path);
                    (path, stamp)
                })
                .collect(),
        }
    }

    fn sources_are_current(&self) -> bool {
        self.rule_sources.iter().all(|(path, stamp)| {
            !matches!(stamp, RuleSourceStamp::Unreadable) && *stamp == RuleSourceStamp::read(path)
        })
    }

    fn requires_refresh(&mut self, path: &Path) -> GitEventRefresh {
        if matches!(self.path_case, GitPathCase::Unresolved)
            || matches!(self.global, GitIgnoreRules::Unreadable)
            || matches!(self.exclude, GitIgnoreRules::Unreadable)
            || self.rule_sources.contains_key(path)
            || path.file_name().is_some_and(|name| name == ".gitignore")
            || !self.sources_are_current()
        {
            return GitEventRefresh::Required;
        }
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return GitEventRefresh::Required;
        };
        if relative.as_os_str().is_empty() || path.to_str().is_none() {
            return GitEventRefresh::Required;
        }
        let TrackedGitPaths::Known(tracked) = &self.tracked else {
            return GitEventRefresh::Required;
        };
        if tracked.contains(&tracked_path_key(relative, self.path_case)) {
            return GitEventRefresh::Required;
        }
        let mut parent = self.root.to_path_buf();
        let mut ancestors = Vec::new();
        let components: Vec<_> = relative.components().collect();
        for (index, component) in components.iter().enumerate() {
            self.directories.entry(parent.clone()).or_insert_with(|| {
                GitIgnoreRules::load(&parent, &parent.join(".gitignore"), self.path_case)
            });
            ancestors.push(parent.clone());
            let child = parent.join(component);
            let directory = index + 1 < components.len() || path.is_dir();
            let rules = ancestors
                .iter()
                .rev()
                .map(|ancestor| &self.directories[ancestor])
                .chain([&self.exclude, &self.global]);
            for rule in rules {
                let GitIgnoreRules::Known(matcher) = rule else {
                    return GitEventRefresh::Required;
                };
                match matcher.matched(&child, directory) {
                    Match::Ignore(_) => return GitEventRefresh::Ignored,
                    Match::Whitelist(_) => break,
                    Match::None => {},
                }
            }
            parent = child;
        }
        GitEventRefresh::Required
    }
}

impl GitIgnoreRules {
    fn load(root: &Path, file: &Path, path_case: GitPathCase) -> Self {
        let mut builder = GitignoreBuilder::new(root);
        if builder
            .case_insensitive(matches!(path_case, GitPathCase::Insensitive))
            .is_err()
        {
            return Self::Unreadable;
        }
        match std::fs::metadata(file) {
            Ok(_) => {
                if builder.add(file).is_some() {
                    return Self::Unreadable;
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(_) => return Self::Unreadable,
        }
        builder.build().map_or(Self::Unreadable, Self::Known)
    }
}

fn tracked_path_key(path: &Path, path_case: GitPathCase) -> PathBuf {
    match path_case {
        GitPathCase::Insensitive => PathBuf::from(path.to_string_lossy().to_ascii_lowercase()),
        GitPathCase::Sensitive | GitPathCase::Unresolved => path.to_path_buf(),
    }
}

fn default_config_paths() -> Vec<PathBuf> {
    let mut sources = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        sources.push(home.join(".gitconfig"));
        sources.push(home.join(".config/git/config"));
        sources.push(home.join(".config/git/ignore"));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        sources.push(PathBuf::from(&xdg).join("git/config"));
        sources.push(PathBuf::from(xdg).join("git/ignore"));
    }
    sources.push(PathBuf::from("/etc/gitconfig"));
    for variable in ["GIT_CONFIG_SYSTEM", "GIT_CONFIG_GLOBAL"] {
        if let Some(path) = std::env::var_os(variable) {
            sources.push(PathBuf::from(path));
        }
    }
    sources
}

fn include_config_sources(root: &Path, sources: &mut Vec<PathBuf>, global: &mut GitIgnoreRules) {
    // Git lists included configuration files. Unknown origins disable suppression.
    match command::git_output_logged(
        root,
        "watcher_config_sources",
        &[
            GIT_CONFIG_COMMAND,
            GIT_NULL_ARG,
            GIT_SHOW_ORIGIN_ARG,
            GIT_LIST_ARG,
        ],
    ) {
        Ok(output) => {
            let mut records = output.trimmed().split('\0');
            while let Some(origin) = records.next() {
                let record = records.next().unwrap_or_default();
                if let Some(file) = origin.strip_prefix("file:") {
                    sources.push(root.join(file));
                } else if !origin.is_empty() {
                    *global = GitIgnoreRules::Unreadable;
                }
                // Includes can create new external sources without a watched event.
                // Keep refreshes when the complete rule source set is unknown.
                if record.starts_with("include.") || record.starts_with("includeif.") {
                    *global = GitIgnoreRules::Unreadable;
                }
            }
        },
        Err(_) => *global = GitIgnoreRules::Unreadable,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on fixture errors")]
mod tests {
    use super::*;

    #[test]
    fn event_visibility_follows_rule_sources_and_directory_replacement() {
        for scenario in ["ignored_rule", "external_rule", "replaced_directory"] {
            let temporary = tempfile::tempdir().expect("fixture");
            let checkout = temporary.path().join("checkout");
            std::fs::create_dir_all(checkout.join("d")).expect("directory");
            crate::test_support::init_git_repo(&checkout);
            let root = AbsolutePath::from(checkout.as_path());
            let git_dir = AbsolutePath::from(checkout.join(".git"));
            let rules = match scenario {
                "external_rule" => temporary.path().join("external-ignore"),
                _ => checkout.join("d/.gitignore"),
            };
            std::fs::write(&rules, "*.log\n").expect("rules");
            if scenario == "ignored_rule" {
                std::fs::write(checkout.join(".gitignore"), "d/.gitignore\n")
                    .expect("ignored rule source");
            }
            if scenario == "external_rule" {
                let status = std::process::Command::new("git")
                    .current_dir(&checkout)
                    .args(["config", "core.excludesfile"])
                    .arg(&rules)
                    .status()
                    .expect("configure excludes");
                assert!(status.success());
            }
            let file = checkout.join("d/a.log");
            std::fs::write(&file, "new file").expect("file");
            let mut cache = GitEventVisibilityCache::default();
            assert_eq!(
                cache.requires_refresh(&root, &git_dir, &file),
                GitEventRefresh::Ignored,
                "{scenario}"
            );
            match scenario {
                "ignored_rule" => {
                    std::fs::remove_file(&rules).expect("remove rules");
                    cache.invalidate(&rules);
                    assert_eq!(
                        cache.requires_refresh(&root, &git_dir, &rules),
                        GitEventRefresh::Required,
                        "ignored rule source"
                    );
                },
                "external_rule" => {
                    std::fs::write(&rules, "").expect("remove external rule");
                    assert_eq!(
                        cache.requires_refresh(&root, &git_dir, &file),
                        GitEventRefresh::Required,
                        "external rule changed"
                    );
                },
                _ => {
                    std::fs::remove_dir_all(checkout.join("d")).expect("remove directory");
                    cache.invalidate(&checkout.join("d"));
                    std::fs::create_dir(checkout.join("d")).expect("replace directory");
                    std::fs::write(&file, "new file").expect("replacement file");
                    assert_eq!(
                        cache.requires_refresh(&root, &git_dir, &file),
                        GitEventRefresh::Required,
                        "directory rules changed"
                    );
                },
            }
        }
    }
}
