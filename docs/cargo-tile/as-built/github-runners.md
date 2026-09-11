# cargo-tile: capture across accounts

## What it is

cargo-tile is a terminal grid of every live cargo invocation on the machine. A process's output belongs to the terminal that started it, so the grid cannot read progress from the process table alone. The capture shim closes that gap: it stands in front of each toolchain's `cargo`, mirrors the run's output into a log, and registers the run for as long as it lives. This feature extends capture to every account on the machine, in particular the self-hosted GitHub Actions runner accounts, so a build started by a runner shows in the grid under that account's name with nothing to configure. Every account writes under its own uid directory below one shared parent, the grid reads all of them, and `sudo cargo-tile install --all-accounts` installs the shim for every account's toolchains in one command.

## How it works

### The shim

`crates/cargo-tile/src/cargo-capture-shim.sh` is POSIX `sh`, embedded into the binary as `SHIM_SOURCE` in `crates/cargo-tile/src/hook.rs`. `cargo-tile install` moves a toolchain's real cargo to `cargo-tile-real` and writes the shim as `cargo`. An installed shim is recognised by `SHIM_MARKER` within its first `SHIM_MARKER_SEARCH_BYTES` bytes.

Per invocation the shim:

- Resolves itself through symlinks and exports `CARGO` naming the shim, so `cargo-clippy` and `cargo-nextest` come back through it. Query subcommands (`metadata`, `--version`, `tile`, `port`, `berth`, and the rest) go straight to the real cargo. Nesting is ancestry: `CARGOTILE_NESTED` carries the enclosing shim's pid and `encloses_this_run` walks one `ps -Ao pid=,ppid=` listing in awk, up to 32 hops.
- Computes its names: `root=/tmp/cargo-tile/$(id -u)`, `pids=$root/state/pids`, a generation `<date>-<uuid>` from `/proc/sys/kernel/random/uuid` or `uuidgen`, the log `run-<generation>-<pid>.log`, the registration `<pid>.<generation>`, and on the no-terminal path a FIFO `state/stderr-<pid>.<generation>`.
- Runs every setup step inside the `setup_capture` subshell under `umask 0022`. It creates the parent when missing and sets it `1777` when it owns it. It creates `$root`, `$root/state` and `$pids`, sets each `0755`, and refuses a symlink or a directory it does not own (`owns_directory`, via `find -user`). It then reaps its own account's dead captures: for every entry in `$pids` whose pid fails `kill -0`, it removes the exact publication, its `.tmp`, its log and its FIFO.
- Records identity. Linux: `boot` is `/proc/sys/kernel/random/boot_id`; `birth` is field 22 of `/proc/$$/stat`, parsed past the last `)`. macOS: `boot` is `sysctl -n kern.bootsessionuuid`; `birth` is `ps -o lstart= -p $$` under `LC_ALL=C TZ=UTC0`, trimmed of BSD column padding, converted by `date -j` to epoch seconds. A field is empty when unobtainable.
- Writes the record with `printf '%s\000'` into an exclusively created `.tmp` file: `cargo-tile-v2`, generation, boot, birth, log basename, `pwd -P` directory, `$HOME` when absolute, argument count, then each argument verbatim. It publishes with `ln` onto `<pid>.<generation>`, and only then creates the log. Any failure removes only the artifacts this invocation owns and `exec`s the real cargo with the original arguments.
- Runs cargo under `script` (util-linux or BSD form) when all three streams are a tty. Otherwise it mirrors stderr through the FIFO with `tee -a`, asks for a progress bar with `CARGO_TERM_PROGRESS_WHEN=always`, and for a `--message-format=json` caller drops standalone `--quiet` and `-q` before `--`. The EXIT trap removes the registration, log and FIFO, so only a run killed outright leaves artifacts behind.

### The shared directory

```
/tmp/cargo-tile/                 CAPTURE_ROOT, mode 1777 (sticky), any owner
  <uid>/                         one per account, owned by that uid, 0755
    run-<generation>-<pid>.log   mirrored output, 0644
    state/                       0755
      pids/<pid>.<generation>    registration, 0644; <name>.tmp is staging
      stderr-<pid>.<generation>  FIFO for the no-terminal path
```

`[capture]` in `config.toml` holds only `auto_install`. The former `roots` key is parsed and ignored (`crates/cargo-tile/src/config.rs`).

### Discovery and the settings pane

`CaptureRoots::from_parent(CAPTURE_ROOT)` in `crates/cargo-tile/src/progress.rs` runs once on the scanner worker. It calls `capture_root::prepare_shared_directory`, which creates the parent and repairs its mode to `1777` through the open handle when the effective user owns it or is root, and canonicalises the parent's ancestors with `canonical_capture_path`. Every scan `discover_accounts` reads the parent with `read_dir` and keeps directories named by a canonical decimal uid, the reader's own uid first, then ascending. Positions are stable across scans (`CaptureRootIndex`). Each is a `CaptureRoot { path, uid, cleanup }` with `cleanup == CaptureCleanup::Here` only when `effective_user()` equals the uid.

`AccountCaptureDirectory::inspect` in `crates/cargo-tile/src/processes.rs` checks the final component with `symlink_metadata` and marks `RootReadStatus::ForeignOwned` when the owner differs from the uid the name claims; such a directory is reported and never read. `SharedCaptureDirectory::inspect` samples the parent into `SharedDirectoryState::{Missing, Shared, NotShared, Unavailable}`. Both travel on `Scan` into `App.shared_directory` and `App.root_status`, and `drain_scans` redraws when either changes. `settings::shared_directory_status` and `settings::capture_root_status` render them with no filesystem access. An account line carries the name (resolved from `sysinfo::Users` on the worker, the uid otherwise), `yours` for the reader's own directory, readable or unreadable, the active capture count, where cleanup happens, cleanup refusals, diagnostics, and per-pid associations.

### The root access layer

`RootScan::open` in `crates/cargo-tile/src/capture_root.rs` opens the account directory, then `state`, then `pids`, each relative to the previous handle with `RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC`, and `fstat`s each into `InspectedDirectoryMetadata { device, inode, owner, group, mode }`. It samples the registration inventory at open (`Inventory::sample`: `RawDir` on Linux, `Dir` elsewhere) into fixed 255-byte names, bounded by `CAPTURE_INVENTORY_LIMIT`. The log inventory is a lazy `OnceLock` used only for legacy annotation. `ScanEntry::read_registration` and `RootScan::read_log` open a validated basename relative to the held handle with `NOFOLLOW | NONBLOCK`, reject non-regular files, and read under `Read::take` caps.

`RootHistory`, thread-local on the worker, pins one `OwnedFd` per root so `RootIncarnation(Uuid)` changes only when the directory object is replaced, and records a `TreeIdentity` per scan so `RootContinuity::Changed` withholds cleanup for one scan after a replacement or a recovered access failure.

Cleanup authority is the private `RootScan::access`, which returns `RootAccess::Owned` only when `cleanup_refusals()` is empty: the root, `state` and `pids` are owned by the effective user with neither `WGRP` nor `WOTH` set, the effective uid was readable, both enumerations are `Enumeration::Complete`, continuity holds, and `revalidate_paths` reopens all three and matches them against the held handles. `OwnedRoot::sweep` removes a registration and its named log as a pair under one `SweepBudget` shared by every root in the scan, revalidating before each unlink.

### Registration parsing and verification

`Registration::parse` in `crates/cargo-tile/src/registration.rs` yields `Versioned(RegistrationCandidate)` for a NUL-framed record with the `cargo-tile-v2` magic, or `Legacy` for tab-separated text. A legacy record can annotate a process row and never sources one. The generation and log fields must be single basenames and the argument count must match. `RegistrationCandidate::verify_observation(pid, &KernelObservation)` is the only route to `VerifiedRegistration`: `Confirmed` when the observation is for the same pid and its `BirthStamp` equals the record's, `Ended` when the kernel proves the writer gone, `Unknown` otherwise.

`registered_runs` in `progress.rs` reads every sampled entry, requires the filename generation to match the record's, verifies against `birth_stamp::observe(pid)`, and retains one `CaptureDiagnostic` per problem. `Capture::scan_root` keys each reading by `CaptureKey { root, pid, incarnation, generation, birth }`, reads each named log once per scan, and pushes a `ConfirmedCapture` for every confirmed record. `sweep_ended` runs only for the reader's own uid directory and only for records verified `Ended`: it rereads the file, requires byte equality with the scan's record, observes the kernel again, and only then returns `SweepDisposition::Remove(log)`.

### Birth stamps

`BirthStamp { boot, birth }` in `crates/cargo-tile/src/birth_stamp/mod.rs` is the comparison unit, and `observe(pid)` returns a `KernelObservation` bound to the pid it read. Linux (`birth_stamp/linux.rs`): the boot id file and field 22 of `/proc/<pid>/stat`; `NotFound` is `Ended`, any other error `Unknown`. macOS (`birth_stamp/macos.rs`): `libc::sysctl` with `KERN_PROC_PID`, decoding the `p_starttime` timeval at offset zero and truncating to whole seconds; boot from `sysctlbyname("kern.bootsessionuuid")`, validated as a UUID. An empty reply or `ESRCH` goes through `process_absence`, which reports `Ended` only when `kill(pid, 0)` returns `ESRCH`. Each platform caches its boot read, failure included, in a `OnceLock` for the process lifetime, and `boot_verification()` reports a cached failure to settings.

### From registration to row

Each scan (`processes::scan`) takes `Capture::take(roots)` before the detail pass, adds every registered pid and its ancestors to the detailed set, then runs `include_registered_processes`, `identify_capture_wrappers` (processes whose argv forwards the registered command, the shim's JSON rewrite included, via `forwards_capture_command`), `collapse_shims` (an outer cargo with exactly one cargo child, same subcommand, same directory by `DirectoryComparison`), `identify_captures`, `select_rows`, `groups` and `associate_status`.

Ownership resolves through `Capture::select(pid)`: the lowest root index first, then within that root one confirmed key, otherwise `Ambiguous`. `Census::captured_run` walks parents up to `PARENT_WALK_LIMIT` for the nearest selection. `Census::direct_capture` yields `DirectAssociation::Direct` only when the walk from the process to the shim pid crosses nothing but observed wrappers; any other intervening process gives the row `CaptureMembership::Enclosing`. A directly captured process row takes its directory and command from the record per field (`row_fields`) before display formatting. A confirmed registration with no directly representing process row sources its own row through `registration_row`: the record's directory, `cargo <args>`, the shim pid, start from the registration file's mtime, cpu and managed `Unavailable(Unproven)`. `registration_directory` shortens the directory to `~` only when the record's `WriterHome` is known and equals the scanner's home, so another account's path is shown in full. `RowProvenance::{Uncaptured, Direct, Enclosing}` carries `CaptureContext { root, incarnation, account }`, and `GroupingIdentity` in `crates/cargo-tile/src/render.rs` groups on that plus the raw `WorkingDirectoryIdentity`, so headings carry `[account]` and rows from different accounts never merge.

### The admin install

`install_all_accounts` in `crates/cargo-tile/src/cli.rs` requires root, reads the account database with `getpwent` (`hook::system_accounts`), prepares the shared directory, and stages a root-owned `0755` copy of the running executable under `/tmp` (`hook::stage_installer`, prefix `cargo-tile-installer-`, removed on drop). `hook::install_accounts` skips accounts without `<home>/.rustup` and runs the staged copy once per account with `HOME` and `RUSTUP_HOME` set and the hidden flag `--account-install-report`. Credentials are applied together in `pre_exec`: `setgroups` with the memberships from `getgrouplist` (`account_groups`, a 32-entry buffer grown when the call returns -1), then `setgid`, then `setuid`. The child prints one `<toolchain>\t<result>` line per toolchain, and `AccountInstallOutcome::from(Output)` folds them into `Installed`, `AlreadyInstalled` or `Skipped(reason)`. No root-owned file operation reaches an account's tree.

Per toolchain, `Hook::install` takes `HookInstallationLock` (exclusive creation of `cargo-tile-shim.lock`, retried, removed on drop) before inspecting `HookState::{Installed, Absent, Repairable, Orphaned}`, and returns `Change::{Installed, Refreshed, AlreadyCurrent, Removed, AlreadyAbsent, Orphaned}`. `Hook::remove` takes the same lock. `Repairable` means the real cargo was moved aside but the shim file is missing, and install rewrites the shim without a second rename. `Orphaned` means a shim is present with no `cargo-tile-real` beside it, and install reports it rather than overwriting. At launch, `Hook::at_startup` runs the same install when `auto_install` is set and the grid shows the tally. The plain `install` reports a failing toolchain and continues, exiting zero so a runner job-start hook never fails over capture setup.

### What the tests prove

`crates/cargo-tile/tests/shim_registration.rs` reaches the sources through `#[path]` modules and drives the built binary under a PTY. `crates/cargo-tile/tests/support/shared_capture.rs` holds the cross-account scenarios. Fixtures write records with Python under the test's own uid; no test needs a second account.

- Injected accounts install through the built binary. The staged installer copies the executable and removes its directory on drop. An installer that cannot start is reported per account, not as a crash. Group resolution matches the account database.
- The reader reports a foreign-owned uid directory as ignored, skips symlinked and non-numeric entries, and shows a new account directory on the next scan.
- The reader creates the parent when missing and repairs a mode it owns.
- A live capture under the real path, scanned through the `/private/tmp` alias, survives the sweep. A record carrying a legacy `{ sec = ` boot is reported `IdentityUnknown` and never unlinked.
- The reader reaps its own ended capture and leaves a foreign-owned directory untouched. The obsolete `roots` key is ignored.
- `reader_regression` covers nested shims, a quiet JSON caller, recovery from ambiguity, locale and timezone variation, legacy registration survival, and removal of an ended staging file.

## Invariants

- The shim never alters what cargo does, prints, or exits with. Every setup failure degrades to `exec "$real" "$@"`.
- The shim is POSIX `sh`: no bashisms, no `readlink -f`, no `local`.
- Publication order and sampling order are one invariant read from two ends. The shim publishes the registration before it creates the log. The reader samples registrations when it opens the root and collects the sample before consulting liveness.
- A sweep removes `<pid>.<generation>` and the log the record names as a pair, decided by fresh kernel evidence for that pid immediately before the unlink, never by age or by name alone. `Unknown` authorizes neither a row nor a deletion.
- Only the reader's own uid directory is swept (`CaptureCleanup::Here`). A foreign-owned directory is reported, not read. Another account's dead captures are reaped by that account's next shim invocation.
- `VerifiedRegistration` is the only proof of identity, and production `KernelObservation` values come only from `observe(pid)`.
- Every directory is opened with `O_NOFOLLOW` relative to a held handle. Ancestor symlinks are resolved once by `canonical_capture_path`; the final component never is.
- `unsafe_code` is denied workspace-wide. The exceptions are `birth_stamp/macos.rs` (sysctl) and the account-database and credential calls in `hook.rs`, each under a per-item `allow(reason)` with a `// SAFETY:` comment.
- Every constant lives in `crates/cargo-tile/src/constants.rs` with a rationale.
- The crate is binary-only. Tests of crate items are inline `#[cfg(test)]` modules, and `crates/cargo-tile/tests/` reaches the sources through `#[path]` modules and drives the built binary.
- The settings pane does no filesystem access; everything it shows was observed on the worker.
- Two uids are never required by a test.

## Calibration and gotchas

- `CAPTURE_SWEEP_LIMIT` is 512 removal attempts per scan across all roots; a pair costs two. `CAPTURE_INVENTORY_LIMIT` is 4096 entries, `CAPTURE_ENTRY_NAME_BYTES` 255, `CAPTURE_REGISTRATION_BYTES` and `RUN_LOG_TAIL_BYTES` 64 KiB, `BIRTH_SYSCTL_MAX_BYTES` 4096, `ACCOUNT_GROUPS_INITIAL_CAPACITY` 32. The worker sleeps `PROCESS_POLL_MILLIS` (250) after each scan, so the scan rate is at most four per second.
- When a live run's log is swept, cargo reopens it through `script` or `tee -a` outside the setup subshell under the caller's umask (`0066` on the runner units), so it lands `0600` and stays unreadable for the rest of that run. That is the reason for the ordering invariant.
- `read_dir` is lazy; collecting is the sample. Keeping the calls in order but dropping the collection reintroduces the defect.
- A single unreadable registration is the ordinary case (a run exits between sample and read) and must not abandon the root. It only withholds the sweep for that scan (`RegistrationEvidence::Incomplete`).
- `effective_user()` and each platform's boot read cache their first result, failure included, for the process lifetime. `IdentityUnknown` (retried next scan) and `IdentityBlockedByBoot` (restart required) stay separate for that reason.
- The ownership rule checks uid and mode bits only. A macOS ACL can grant another account write access without changing those bits; `InspectedDirectoryMetadata::refusals` documents the gap and it remains open.
- macOS: `kern.boottime` is recomputed on clock corrections, so the boot field is the session UUID. `BirthStamp::compare` returns `Unknown`, never `Ended`, for a legacy `{ sec = ` boot string against a present process. The macOS birth truncates to whole seconds; the generation, not the birth, separates two invocations born in one second. `ps -o lstart=` is padded to column width and rendered through `localtime`, so the shim trims it and reads under `LC_ALL=C TZ=UTC0`. `/tmp` is `/private/tmp`; `DirectoryComparison::between` compares aliased directories by device and inode, and relative paths never establish identity.
- A runner service whose PATH lists Nix or Homebrew coreutils ahead of `/bin` resolves `date` to the GNU binary, which rejects `-j`. The shim tries the BSD form and falls back to `date -u -d`, so the birth field is filled under either binary. An empty birth field means the tile can neither draw nor delete that registration.
- The macOS boot field changed from a timeval to a UUID with no magic bump. A tile from the previous build treats such records as `Unknown`, so nothing is unlinked, but it does not report the skew.
- The state column draws `blocked` only for `waiting for file lock on build directory`; a package-cache lock wait is ignored by design.
- sysinfo: a CPU rate is trustworthy only when the previous accumulated counter is positive; `accumulated_cpu_time()` is quantized; start times are whole seconds, which is why registration-sourced rows use the registration mtime. `UpdateKind::OnlyIfNotSet` never re-reads a populated field, so the detail pass builds a fresh `System` per scan.
- `uninstall` still stops at the first toolchain that errors; `install` reports and continues.
- The settings popup does not scroll, so enough account lines push rows off the bottom.
- `reader_keeps_application_spawned_cargo_when_run_is_excluded` is timing-sensitive: about one flake in three package runs, green on rerun.
- The staged installer relies on the parent of `CAPTURE_ROOT` being sticky and root-owned. `getgrouplist` growth past 32 entries is untested on macOS.
- A runner unit with a private `/tmp` needs `BindPaths=/tmp/cargo-tile`. The Mac launchd plist still exports `CARGO_TILE_ROOT`, which nothing reads any more.

## Why

- One shared parent with per-uid children replaces a configured root list: the shim, reader and installer all know one machine location, and a runner account needs no configuration. Sticky `1777` lets any account create its directory while only the owner can replace it.
- Cleanup is per account because a non-owning reader cannot unlink under a `0755` directory, and every cross-uid removal attempt would fail on every entry, permanently. The shim reaps its own predecessors instead.
- Hard-link publication rather than `mv -f`: a rename silently replaces a name a live run still owns, while `ln` fails and a failure is a setup failure like any other.
- NUL framing rather than a `<cwd>\tcargo <args>` line: space-joined argv loses argument boundaries, and a tab or newline inside an argument breaks the framing.
- `/proc/$$/stat` rather than `/proc/self/stat`: inside the setup subshell `self` is the short-lived child, not the pid the registration is filed under.
- Verification by kernel evidence before deletion: exclusive creation prevents a name being taken while a record holds it, and does not prevent the same name being published again once removed, so deleting by exact name is not safe on its own. Age cannot establish that a paused writer ended, so a staging file is eligible only when the kernel confirms its writer gone.
- `libc::sysctl` on macOS rather than `sysctl(8)` or `ps(1)`: a subprocess per registration per scan at four scans a second. rustix has no sysctl binding, and sysinfo's start times are rounded on Linux and zero on the macOS fallback.
- The boot session UUID rather than `kern.boottime`: the latter follows clock corrections, so every live registration mismatched and was swept within a scan.
- `kill(pid, 0)` before calling a process ended: an empty `KERN_PROC_PID` reply can mean a hidden record, not an exited process.
- One ownership rule on both platforms rather than a platform gate returning foreign for every non-Linux root, under which no Mac would ever sweep. A trusted-prefix list preserving macOS `/tmp` became unnecessary with the uniform rule.
- `geteuid` from `rustix::process` rather than a process-table view, which depends on process visibility.
- A staged root-owned installer copy under `/tmp` rather than the administrator's own binary: the account child must execute it whatever the permissions on the administrator's home, and a sticky parent stops another account replacing it.
- Credentials applied together in `pre_exec`, `setgroups` first: `Command::uid` alone leaves the child without the account's supplementary groups, so a toolchain the account reaches through a group would fail.
- Registration mtime as the run start: it is the one value the reader already has across a uid boundary.
- Restoring `cargo-tile-real` before installing over it was ruled out; the round trip is the window in which a second installer saves a shim as the real cargo. A file-locking dependency was ruled out in favour of the existing exclusive-create idiom.
- Changing the runner unit's `UMask=0066` was ruled out; it reaches every file the runner writes, credentials included. Recursive mode correction beneath the root was ruled out; one level, only directories the shim owns.
- Canonicalising the whole account path was ruled out; it would resolve the final component and defeat the `O_NOFOLLOW` check.
- A second row for a pid confirmed under a non-selected root was ruled out; one live invocation is one tile, and `Ambiguous` means no owner rather than pick one.
- Letting `CaptureKey`'s derived `Ord` decide ownership was ruled out; a filename string would pick the live generation among equally confirmed registrations.
- Resolving the owner uid to a name at render time was ruled out; the worker resolves it and grouping keys on the number.
- Capturing `cargo tile`, `cargo port` and `cargo berth` was ruled out; the first two would run a terminal UI under `script`, and the third fires several times a second with no build to mirror.
