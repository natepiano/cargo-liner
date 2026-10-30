//! Stat known script inputs without walking their trees on each refresh.

use std::collections::BTreeMap;
use std::fs;
use std::fs::Metadata;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use serde_json::Value;

/// Metadata at the filesystem boundary; absence is an input revision too.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::census) enum FileRevision {
    Missing,
    Present {
        modified: SystemTime,
        size:     u64,
        #[cfg(unix)]
        device:   u64,
        #[cfg(unix)]
        inode:    u64,
        #[cfg(unix)]
        changed:  (i64, i64),
    },
}

impl FileRevision {
    fn at(path: &Path) -> Self {
        fs::metadata(path).map_or(Self::Missing, |metadata| Self::of(&metadata))
    }

    pub(in crate::census) fn of(metadata: &Metadata) -> Self {
        let Ok(modified) = metadata.modified() else {
            return Self::Missing;
        };
        Self::Present {
            modified,
            size: metadata.len(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

/// A finite pattern names input files, never an unrestricted recursive walk.
struct InputPattern {
    root:  PathBuf,
    names: Vec<InputName>,
}

enum InputName {
    Exact(String),
    Prefix(String),
    Suffix(String),
}

impl InputName {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Exact(wanted) => name == wanted,
            Self::Prefix(prefix) => name.starts_with(prefix),
            Self::Suffix(suffix) => name.ends_with(suffix),
        }
    }
}

/// Directory entries are listed only after that directory's metadata changes.
#[derive(Default)]
struct InputDirectories(BTreeMap<PathBuf, (FileRevision, Vec<PathBuf>)>);

impl InputDirectories {
    fn matching(&mut self, root: &Path, name: &InputName) -> Vec<PathBuf> {
        if let InputName::Exact(name) = name {
            return vec![root.join(name)];
        }
        let revision = FileRevision::at(root);
        let entry = self
            .0
            .entry(root.to_owned())
            .or_insert_with(|| (FileRevision::Missing, Vec::new()));
        if entry.0 != revision {
            entry.1 = fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .collect();
            entry.0 = revision;
        }
        entry
            .1
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|file| name.matches(&file.to_string_lossy()))
            })
            .cloned()
            .collect()
    }
}

/// An answer retained until a file revision or a non-file input deadline changes.
pub(in crate::census) struct InputDrivenCommand<T> {
    paths:       Vec<PathBuf>,
    patterns:    Vec<InputPattern>,
    directories: InputDirectories,
    fallback:    Duration,
    clock:       ScriptClockInputs,
    answer:      RetainedCommandAnswer<T>,
}

enum RetainedCommandAnswer<T> {
    Unasked,
    Answered {
        value:          T,
        revisions:      BTreeMap<PathBuf, FileRevision>,
        clock_revision: usize,
        checked:        Instant,
    },
}

impl<T: Clone> InputDrivenCommand<T> {
    pub(in crate::census) fn new(paths: Vec<PathBuf>, fallback: Duration) -> Self {
        Self {
            paths,
            patterns: Vec::new(),
            directories: InputDirectories::default(),
            fallback,
            clock: ScriptClockInputs::None,
            answer: RetainedCommandAnswer::Unasked,
        }
    }

    fn pattern(&mut self, root: PathBuf, names: Vec<InputName>) {
        self.patterns.push(InputPattern { root, names });
    }

    fn revisions(&mut self) -> BTreeMap<PathBuf, FileRevision> {
        let mut paths = self.paths.clone();
        let mut used_directories = std::collections::BTreeSet::new();
        for pattern in &self.patterns {
            let mut level = vec![pattern.root.clone()];
            for name in &pattern.names {
                level = level
                    .iter()
                    .flat_map(|path| {
                        if !matches!(name, InputName::Exact(_)) {
                            used_directories.insert(path.clone());
                        }
                        self.directories.matching(path, name)
                    })
                    .collect();
            }
            paths.extend(level);
        }
        self.directories
            .0
            .retain(|path, _| used_directories.contains(path));
        paths
            .into_iter()
            .map(|path| {
                let revision = FileRevision::at(&path);
                (path, revision)
            })
            .collect()
    }

    pub(in crate::census) fn read(&mut self, now: Instant, read: impl FnOnce() -> T) -> T {
        let revisions = self.revisions();
        let clock_revision = self.clock.revision(chrono::Utc::now().timestamp());
        if let RetainedCommandAnswer::Answered {
            value,
            revisions: previous,
            clock_revision: previous_clock,
            checked,
        } = &self.answer
            && *previous == revisions
            && *previous_clock == clock_revision
            && now.saturating_duration_since(*checked) < self.fallback
        {
            return value.clone();
        }
        let value = read();
        self.answer = RetainedCommandAnswer::Answered {
            value: value.clone(),
            revisions,
            clock_revision,
            checked: now,
        };
        value
    }

    /// Failed commands retry through the caller's existing cadence.
    pub(in crate::census) fn invalidate(&mut self) { self.answer = RetainedCommandAnswer::Unasked; }

    pub(in crate::census) fn for_command(command: &str, _cadence: Duration) -> Self {
        let Some(home) = dirs::home_dir() else {
            return Self::new(Vec::new(), Duration::ZERO);
        };
        Self::for_command_at(command, &home)
    }

    fn for_command_at(command: &str, home: &Path) -> Self {
        let words: Vec<_> = command.split_whitespace().collect();
        let hierarchy = words.len() == 3
            && words[0].ends_with("/.claude/scripts/lib/py")
            && words[1].ends_with("/.claude/scripts/production/production_tree.py")
            && words[2] == "--json";
        let accounts = words.len() == 5
            && words[0].ends_with("/.claude/scripts/lib/py")
            && words[1].ends_with("/.claude/scripts/accounts/accounts.py")
            && words[2] == "status"
            && matches!(
                (words[3], words[4]),
                ("--stored", "--json") | ("--json", "--stored")
            );
        if !hierarchy && !accounts {
            return Self::new(Vec::new(), Duration::ZERO);
        }
        // Live herdr marks, process liveness, keychain state and external links have
        // no complete file input contract. Recheck these on a five-minute fallback.
        let mut inputs = Self::new(Vec::new(), Duration::from_secs(300));
        let scripts = home.join(".claude/scripts");
        for directory in [
            "production",
            "accounts",
            "whoami",
            "message",
            "lib",
            "astra",
            "guilds",
        ] {
            inputs.pattern(
                scripts.join(directory),
                vec![InputName::Suffix(".py".into())],
            );
        }
        if hierarchy {
            inputs.hierarchy_inputs(home);
        }
        if accounts {
            inputs.accounts_inputs(home);
        }
        inputs
    }

    fn hierarchy_inputs(&mut self, home: &Path) {
        self.paths
            .push(home.join(".claude/scripts/direct/board.sh"));
        for root in ["/tmp/claude/unit", "/tmp/claude/delegate"] {
            for file in [
                "director",
                "mesh_roster.json",
                "mesh_server.json",
                "workers",
                "board.log",
                "progress_history_state.json",
            ] {
                self.pattern(
                    root.into(),
                    vec![
                        InputName::Prefix(String::new()),
                        InputName::Exact(file.into()),
                    ],
                );
            }
            self.pattern(
                root.into(),
                vec![
                    InputName::Prefix(String::new()),
                    InputName::Prefix("impl_worker_".into()),
                ],
            );
        }
        self.pattern(
            env_path("NOTIFIER_SESSIONS_DIR", home.join(".claude/sessions")),
            vec![InputName::Suffix(".json".into())],
        );
        self.pattern(
            env_path("NOTIFIER_STATE_DIR", home.join(".local/state/notifier")),
            vec![
                InputName::Prefix("showrunner-".into()),
                InputName::Exact("conf".into()),
            ],
        );
        self.pattern(
            env_path("SHOWRUNNER_STATE_DIR", home.join(".local/state/showrunner")),
            vec![
                InputName::Prefix(String::new()),
                InputName::Exact("parent".into()),
            ],
        );
        self.pattern(
            env_path("REVIEW_POOL_STATE", home.join(".local/state/review-pool")),
            vec![
                InputName::Prefix(String::new()),
                InputName::Exact("pool.json".into()),
            ],
        );
        self.pattern(
            env_path("ASTRA_ROLES_STATE", home.join(".local/state/astra/roles")),
            vec![InputName::Suffix(".json".into())],
        );
        let guilds = env_path("GUILD_ROLES_STATE", home.join(".local/state/guilds/roles"));
        self.pattern(
            guilds.clone(),
            vec![
                InputName::Prefix(String::new()),
                InputName::Exact("design.json".into()),
            ],
        );
        self.pattern(
            guilds,
            vec![
                InputName::Prefix(String::new()),
                InputName::Exact("mesh".into()),
                InputName::Exact("mesh_roster.json".into()),
            ],
        );
        self.paths.push(env_path(
            "SHOWRUNNERS_CONFIG",
            home.join(".claude/config/showrunners.json"),
        ));
    }

    fn accounts_inputs(&mut self, home: &Path) {
        for file in ["groups.json", "role_map_cache.json", "login_renewals.json"] {
            self.paths
                .push(home.join(".local/state/accounts").join(file));
        }
        let usage = home.join(".local/state/agent-notes/usage.json");
        self.paths.push(usage.clone());
        self.clock = ScriptClockInputs::Stored {
            paths:      vec![
                usage,
                home.join(".local/state/accounts/groups.json"),
                home.join(".local/state/accounts/role_map_cache.json"),
            ],
            revisions:  BTreeMap::new(),
            boundaries: Vec::new(),
        };
        self.paths.push(home.join(".claude/config/accounts.json"));
        self.paths.push(home.join(".claude.json"));
        self.pattern(
            env_path("AGENT_NOTES_DIR", home.join("rust/hanadocs/agents")),
            vec![InputName::Suffix(".md".into())],
        );
        for prefix in [".claude", ".codex"] {
            // One level inside account homes: no transcripts, sessions or databases
            // are traversed. File stats also detect replaced credential symlinks.
            self.pattern(
                home.to_owned(),
                vec![
                    InputName::Prefix(prefix.into()),
                    InputName::Prefix(String::new()),
                ],
            );
        }
    }
}

/// Quota resets and expiring role routes are inputs even without a file write.
enum ScriptClockInputs {
    None,
    Stored {
        paths:      Vec<PathBuf>,
        revisions:  BTreeMap<PathBuf, FileRevision>,
        boundaries: Vec<i64>,
    },
}

impl ScriptClockInputs {
    fn revision(&mut self, unix_now: i64) -> usize {
        let Self::Stored {
            paths,
            revisions,
            boundaries,
        } = self
        else {
            return 0;
        };
        let current: BTreeMap<_, _> = paths
            .iter()
            .map(|path| (path.clone(), FileRevision::at(path)))
            .collect();
        if *revisions != current {
            boundaries.clear();
            for path in paths {
                if let Ok(bytes) = fs::read(path)
                    && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    collect_time_inputs(&value, boundaries);
                }
            }
            *revisions = current;
        }
        boundaries
            .iter()
            .filter(|boundary| **boundary <= unix_now)
            .count()
    }
}

fn collect_time_inputs(value: &Value, boundaries: &mut Vec<i64>) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if matches!(name.as_str(), "resets" | "until")
                    && let Some(text) = value.as_str()
                    && let Ok(time) = chrono::DateTime::parse_from_rfc3339(text)
                {
                    boundaries.push(time.timestamp());
                }
                collect_time_inputs(value, boundaries);
            }
        },
        Value::Array(values) => {
            for value in values {
                collect_time_inputs(value, boundaries);
            }
        },
        _ => {},
    }
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(name).map_or(default, |value| {
        let path = PathBuf::from(value);
        path.strip_prefix("~")
            .ok()
            .and_then(|rest| dirs::home_dir().map(|home| home.join(rest)))
            .unwrap_or(path)
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;

    use super::*;

    #[test]
    fn idle_scripts_reuse_unchanged_inputs_and_run_once_after_a_change()
    -> Result<(), Box<dyn Error>> {
        for script in ["production_tree.py", "accounts.py"] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join(script);
            fs::write(&path, "first")?;
            let mut command = InputDrivenCommand::new(vec![path.clone()], Duration::from_secs(300));
            let runs = Cell::new(0);
            let run = || {
                runs.set(runs.get() + 1);
                runs.get()
            };
            let now = Instant::now();
            assert_eq!(command.read(now, run), 1);
            assert_eq!(
                command.read(now + Duration::from_secs(60), run),
                1,
                "{script}: unchanged inputs reuse the answer"
            );
            fs::write(&path, "changed input")?;
            assert_eq!(command.read(now + Duration::from_secs(61), run), 2);
            assert_eq!(command.read(now + Duration::from_secs(62), run), 2);
            assert_eq!(runs.get(), 2, "{script}: one command per input revision");
        }
        Ok(())
    }

    #[test]
    fn idle_scripts_discover_new_and_removed_inputs_without_recursive_walks()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let mut command = InputDrivenCommand::new(Vec::new(), Duration::from_secs(300));
        command.pattern(
            directory.path().to_owned(),
            vec![InputName::Suffix(".json".into())],
        );
        let runs = Cell::new(0);
        let run = || {
            runs.set(runs.get() + 1);
            runs.get()
        };
        let now = Instant::now();
        assert_eq!(command.read(now, run), 1);
        assert_eq!(command.read(now, run), 1);
        fs::write(directory.path().join("new.json"), "new")?;
        assert_eq!(command.read(now, run), 2);
        fs::remove_file(directory.path().join("new.json"))?;
        assert_eq!(command.read(now, run), 3);
        assert_eq!(command.read(now, run), 3);
        assert_eq!(
            command.read(now + Duration::from_secs(300), run),
            4,
            "unobservable input fallback"
        );
        Ok(())
    }
    #[test]
    fn idle_scripts_clock_inputs_expire_without_rereading_unchanged_usage()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("usage.json");
        fs::write(
            &path,
            r#"{"account":{"weekly":{"resets":"1970-01-01T00:01:40Z"},"five_hour":{"resets":"1970-01-01T00:00:50Z"}}}"#,
        )?;
        let mut clock = ScriptClockInputs::Stored {
            paths:      vec![path],
            revisions:  BTreeMap::new(),
            boundaries: Vec::new(),
        };
        assert_eq!(clock.revision(49), 0);
        assert_eq!(clock.revision(50), 1);
        assert_eq!(clock.revision(99), 1);
        assert_eq!(clock.revision(100), 2);
        Ok(())
    }

    #[test]
    fn idle_scripts_custom_commands_keep_their_refresh_and_failures_retry() {
        let now = Instant::now();
        let mut command =
            InputDrivenCommand::for_command("printf accounts.py", Duration::from_secs(30));
        let runs = Cell::new(0);
        let run = || {
            runs.set(runs.get() + 1);
            runs.get()
        };
        assert_eq!(command.read(now, run), 1);
        assert_eq!(command.read(now + Duration::from_secs(30), run), 2);
        command.invalidate();
        assert_eq!(command.read(now + Duration::from_secs(31), run), 3);
    }

    #[test]
    fn idle_scripts_builtin_commands_watch_identity_renewals_and_board_source()
    -> Result<(), Box<dyn Error>> {
        let home = tempfile::tempdir()?;
        for (script, args, relative) in [
            (
                "accounts/accounts.py",
                "status --json --stored",
                ".claude.json",
            ),
            (
                "accounts/accounts.py",
                "status --json --stored",
                ".local/state/accounts/login_renewals.json",
            ),
            (
                "production/production_tree.py",
                "--json",
                ".claude/scripts/direct/board.sh",
            ),
        ] {
            let path = home.path().join(relative);
            fs::create_dir_all(path.parent().ok_or("fixture parent")?)?;
            fs::write(&path, "before")?;
            let text = format!("~/.claude/scripts/lib/py ~/.claude/scripts/{script} {args}");
            let mut command = InputDrivenCommand::for_command_at(&text, home.path());
            let runs = Cell::new(0);
            let run = || {
                runs.set(runs.get() + 1);
                runs.get()
            };
            let now = Instant::now();
            assert_eq!(command.read(now, run), 1);
            assert_eq!(command.read(now, run), 1);
            fs::write(&path, "after input change")?;
            assert_eq!(
                command.read(now, run),
                2,
                "{relative} is a readable script input"
            );
            assert_eq!(command.read(now, run), 2);
        }
        Ok(())
    }

    #[test]
    fn idle_scripts_detect_equal_size_rewrites_and_replacements() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("input.json");
        fs::write(&path, "one")?;
        let modified = fs::metadata(&path)?.modified()?;
        let mut command = InputDrivenCommand::new(vec![path.clone()], Duration::from_secs(300));
        let runs = Cell::new(0);
        let run = || {
            runs.set(runs.get() + 1);
            runs.get()
        };
        let now = Instant::now();
        assert_eq!(command.read(now, run), 1);
        for (text, replace, expected) in [("two", false, 2), ("six", true, 3)] {
            if replace {
                let replacement = directory.path().join("replacement");
                fs::write(&replacement, text)?;
                fs::rename(replacement, &path)?;
            } else {
                fs::write(&path, text)?;
            }
            fs::File::options()
                .write(true)
                .open(&path)?
                .set_times(fs::FileTimes::new().set_modified(modified))?;
            assert_eq!(
                command.read(now, run),
                expected,
                "restoring mtime cannot hide a new revision"
            );
            assert_eq!(command.read(now, run), expected);
        }
        Ok(())
    }

    #[test]
    fn idle_scripts_prune_removed_directory_listings() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let nested = directory.path().join("run");
        fs::create_dir(&nested)?;
        fs::write(nested.join("input.json"), "input")?;
        let mut command = InputDrivenCommand::new(Vec::new(), Duration::from_secs(300));
        command.pattern(
            directory.path().to_owned(),
            vec![
                InputName::Prefix(String::new()),
                InputName::Suffix(".json".into()),
            ],
        );
        command.read(Instant::now(), || ());
        assert!(command.directories.0.contains_key(&nested));
        fs::remove_dir_all(&nested)?;
        command.read(Instant::now(), || ());
        assert!(
            !command.directories.0.contains_key(&nested),
            "removed run directories release their cached listings"
        );
        assert!(command.directories.0.contains_key(directory.path()));
        Ok(())
    }
}
