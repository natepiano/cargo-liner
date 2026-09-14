# cargo-tile: capture across accounts

## What it is

cargo-tile is a terminal grid of every live cargo invocation on the machine. A process's output belongs to the terminal that started it, so the grid cannot read progress from the process table alone. The capture shim closes that gap: it stands in front of each toolchain's `cargo`, mirrors the run's output into a log, and registers the run for as long as it lives. Capture extends to every account on the machine, in particular the self-hosted GitHub Actions runner accounts (`hana-linux-1` and `hana-linux-2` on Linux, `hana-ci` on macOS), so a build started by a runner shows in the grid under that account's name with nothing to configure. Every account writes under its own uid directory below one shared parent, the grid reads all of them, and `sudo cargo tile install --all-accounts` installs the shim for every account's toolchains in one command.

The pieces:
- A POSIX `sh` shim that publishes a `cargo-tile-v3` NUL-framed registration before its log.
- A shared `/tmp/cargo-tile/<uid>` directory, read for every account and swept only for the reader's own, with each file proved on its own before unlink.
- Registration parsing bound to kernel birth stamps, with a diagnostic for a newer framing version.
- `install`, `uninstall` and `status` for every account, reporting each toolchain by name. A Darwin account keeps its full group membership past the 16-group `setgroups` limit.
- Cargo rows that report the CPU time their invocation causes, including reaped children and compiler-cache work.
- A Settings pane in which every line can be scrolled to.
- A Linux desktop backdrop that does not spawn a process or open a session-bus connection on every capture tick.
- cargo-berth readers that tolerate the `merge_extent_observed` journal record, with evidence decisions that match the live answer.

## How it works

### The shim

`crates/cargo-tile/src/cargo-capture-shim.sh` is POSIX `sh`, embedded into the binary as `SHIM_SOURCE` in `crates/cargo-tile/src/hook.rs`. `cargo-tile install` moves a toolchain's real cargo to `cargo-tile-real` and writes the shim as `cargo`. An installed shim is recognised by `SHIM_MARKER` within its first `SHIM_MARKER_SEARCH_BYTES` bytes, and its line 3 is `# cargo-tile-shim-version: 3`.

Per invocation the shim:

- Resolves itself through symlinks and exports `CARGO` naming the shim, so `cargo-clippy` and `cargo-nextest` come back through it. Query subcommands (`metadata`, `--version`, `tile`, `port`, `berth`, and the rest) go straight to the real cargo. Nesting is ancestry: `CARGOTILE_NESTED` carries the enclosing shim's pid and `encloses_this_run` walks one `ps -Ao pid=,ppid=` listing in awk, up to 32 hops.
- Computes its names: `root=/tmp/cargo-tile/$(id -u)`, `pids=$root/state/pids`, a generation `<date>-<uuid>` from `/proc/sys/kernel/random/uuid` or `uuidgen`, the log `run-<generation>-<pid>.log`, the registration `<pid>.<generation>`, and on the no-terminal path a FIFO `state/stderr-<pid>.<generation>`.
- Runs every setup step inside the `setup_capture` subshell under `umask 0022`. It creates the parent when missing and sets it `1777` when it owns it. It creates `$root`, `$root/state` and `$pids`, sets each `0755`, and refuses a symlink or a directory it does not own (`owns_directory`, via `find -user`). It then reaps its own account's dead captures: for every entry in `$pids` whose pid fails `kill -0`, it removes the exact publication, its `.tmp`, its log and its FIFO.
- Records identity. Linux: `boot` is `/proc/sys/kernel/random/boot_id`; `birth` is field 22 of `/proc/$$/stat`, parsed past the last `)`. macOS: `boot` is `sysctl -n kern.bootsessionuuid`; `birth` is `ps -o lstart= -p $$` under `LC_ALL=C TZ=UTC0`, trimmed of BSD column padding, converted by `date -j` to epoch seconds. A field is empty when unobtainable.
- Writes the record with `printf '%s\000'` into an exclusively created `.tmp` file: `cargo-tile-v3`, generation, boot, birth, log basename, `pwd -P` directory, `$HOME` when absolute, argument count, then each argument verbatim. It publishes with `ln` onto `<pid>.<generation>`, and only then creates the log. Any failure removes only the artifacts this invocation owns and `exec`s the real cargo with the original arguments.
- Runs cargo under `script` (util-linux or BSD form) when all three streams are a tty. Otherwise it mirrors stderr through the FIFO with `tee -a`, asks for a progress bar with `CARGO_TERM_PROGRESS_WHEN=always`, and for a `--message-format=json` caller drops standalone `--quiet` and `-q` before `--`. The EXIT trap removes the registration, log and FIFO, so only a run killed outright leaves artifacts behind.

A `cargo install` goes down the capture path the same way `cargo build` does: the shim's `case $first` classifier, the `Building [...] n/m` counter, and `heading_gauge`. No install-specific code exists.

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

`CaptureRoots::from_parent(CAPTURE_ROOT)` in `crates/cargo-tile/src/progress/capture_roots.rs` runs once on the scanner worker. It calls `root_scan::prepare_shared_directory`, which creates the parent and repairs its mode to `1777` through the open handle when the effective user owns it or is root, and canonicalises the parent's ancestors with `canonical_capture_path`. Every scan `discover_accounts` reads the parent with `read_dir` and keeps directories named by a canonical decimal uid, the reader's own uid first, then ascending. Positions are stable across scans (`CaptureRootIndex`). Each is a `CaptureRoot { path, uid, cleanup }` with `cleanup == CaptureCleanup::Here` only when `effective_user()` equals the uid.

`AccountCaptureDirectory::inspect` checks the final component with `symlink_metadata` and marks `RootReadStatus::ForeignOwned` when the owner differs from the uid the name claims; such a directory is reported and never read. `SharedCaptureDirectory::inspect` samples the parent into `SharedDirectoryState::{Missing, Shared, NotShared, Unavailable}`. Both travel on `Scan` into `App.shared_directory` and `App.root_status`, and `drain_scans` redraws when either changes. `settings::shared_directory_status` and `settings::capture_root_status` render them with no filesystem access. An account line carries the name (resolved from `sysinfo::Users` on the worker, the uid otherwise), `yours` for the reader's own directory, readable or unreadable, the active capture count, read diagnostics, and per-pid associations. Cleanup conditions never reach Settings. `CaptureDiagnostic::EnumerationIncomplete` is a per-root read diagnostic and is shown there.

### Progress capture modules (`crates/cargo-tile/src/progress/`)

`mod.rs` holds `Progress` and re-exports nothing.

| File | Contents |
| --- | --- |
| `capture.rs` | `CaptureRootIndex`, `CaptureKey`, `CaptureGeneration`, `CaptureSelection`, `ConfirmedCapture`, `Capture`, `LogWriter` |
| `capture_roots.rs` | `CaptureRoot`, `CaptureCleanup`, `CaptureParent`, `CaptureRoots::from_parent`, `AccountCaptureDirectory`, `RootReadStatus`, `AccountName` |
| `capture_read.rs` | `Phase`, `RunState`, `CaptureLookup`, `CaptureRead`, `Counter`, and the parser outcomes |
| `capture_diagnostic.rs` | `CaptureDiagnostic`, `CaptureFailure`, `PathFailure` |
| `registered_runs.rs` | `RegisteredRuns`, `RegisteredRun`, `RegistrationName`, `registered_runs` |

Parser outcomes in `capture_read.rs`:
- `CurrentProgress::{Active, RetiredCounter, Unrecognized}`, `TailCounter` and `ParsedCounter`.
- `LeadingNumber::{Digits, NoLeadingDigits, Overflow}`.
- `CaptureRead` folds `RetiredCounter` and `Unrecognized` into `NoCurrentProgress`.
- `impl From<&CaptureLookup> for CounterState` in `render.rs` is the only conversion from a lookup to the gauge.

### The root access layer and sweeps (`crates/cargo-tile/src/root_scan/`)

Files: `mod.rs` (`RootScan`), `inspected_directory.rs` (directory inspection, `Inventory`, `ScanEntry`, `read_registration_file`), `root_history.rs` (`RootHistory`, `RootIncarnation`, `TreeIdentity`, `RootContinuity`), `sweep_authority.rs` (sweep policy).

`RootScan::open` opens the account directory, then `state`, then `pids`, each relative to the previous handle with `RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC`, and `fstat`s each into `InspectedDirectoryMetadata { device, inode, owner, mode }`. It samples the registration inventory at open (`Inventory::sample`: `RawDir` on Linux, `Dir` on Darwin) into fixed 255-byte names, bounded by `CAPTURE_INVENTORY_LIMIT`. The log inventory is a lazy `OnceLock` initialised only by legacy annotation. `ScanEntry::read_registration` and `RootScan::read_log` open a validated basename relative to the held handle with `NOFOLLOW | NONBLOCK`, reject non-regular files, and read under `Read::take` caps.

`RootHistory`, thread-local on the worker, pins one `OwnedFd` per root so `RootIncarnation(Uuid)` changes only when the directory object is replaced, and records a `TreeIdentity` per scan so `RootContinuity::Changed` withholds cleanup for one scan after a replacement or a recovered access failure.

Sweeping:
- `RootScan::sweep(&self, &mut SweepBudget, impl FnMut(ScanEntry<'_>) -> SweepDisposition) -> SweepCounts`. One `SweepBudget` is shared across the scan. `SweepCounts` is not re-exported from the hub.
- The private `access(EffectiveUser) -> Result<SweepAuthority<'_>, SweepAdmissionRefusal>` admits a sweep when:
  - the effective uid equals the root owner (otherwise `ForeignRoot`, or `EffectiveUserUnavailable`);
  - `revalidate_paths` reopens root, `state` and `pids` and matches dev/ino/owner against the held handles;
  - root, `state` and `pids` are all owned by that uid;
  - continuity is not `Changed` (otherwise `AccessFailure`).
- `SweepAdmissionRefusal` is payload-free with exactly those three outcomes. A refusal becomes `SweepCounts::unavailable_root()`: no callback runs and no budget is spent. Directory write grants decide neither admission nor candidate eligibility.
- `SweepAuthority::sweep` counts each non-`Complete` `Enumeration` in the registration sample and in any sampled log inventory into `incomplete_inventories`, then proceeds. A partial inventory still sweeps the pairs it established.
- `sweep_pair` proves the registration and then its log, each through `SweepFileInspection::inspect`:
  1. `openat` with `NOFOLLOW|NONBLOCK|CLOEXEC` relative to the retained directory handle.
  2. `fstat`: the file must be regular, owned by the effective uid, have no group or other write bit, and have `st_nlink == 1`.

  It then:
  1. Re-runs the callback with `RegistrationReadPurpose::SweepRevalidation`.
  2. Revalidates paths and charges two budget units for the pair.
  3. `SweepEligibleFile::revalidate` compares the held dev/ino against `statat(SYMLINK_NOFOLLOW)` right before each `unlinkat`.
  4. Unlinks the log first, re-checks the callback and paths, then unlinks the registration.
  5. A missing log is allowed.
- A failed pair is preserved and increments `skipped_pair_attempts`.
- `SweepCounts { removed_files, skipped_pair_attempts, incomplete_inventories, unavailable_roots }` stays on the worker. Only `cfg(test)` accessors read it, and nothing about cleanup reaches Settings.
- The module contains no ACL inspector. Its only `unsafe` is the test FIFO fixture in `inspected_directory.rs`.

### Registration parsing and verification

`Registration::parse` in `crates/cargo-tile/src/registration.rs` yields `Versioned(RegistrationCandidate)` for a NUL-framed record, or `Legacy` for tab-separated text. It calls `check_version(bytes)` first, which scans at most `REGISTRATION_VERSION_HEADER_BYTES + 1` bytes for the NUL separator:
- A `cargo-tile-v3` or `cargo-tile-v2` header returns `Ok`.
- `cargo-tile-v<digits>` above `SUPPORTED_REGISTRATION_VERSION` (3) returns `UnsupportedVersion { encountered }`, whatever the payload size.
- A header with no separator returns `Framing`, and the record falls through to the legacy parse.

Size (`TooLarge` above `CAPTURE_REGISTRATION_BYTES`) is checked after the version. The descriptor read takes one byte past the cap and returns the bytes it already holds, so a newer oversized record still reaches version dispatch.

A legacy record can annotate a process row and never sources one. The generation and log fields must be single basenames and the argument count must match. `RegistrationCandidate::verify_observation(pid, &KernelObservation)` is the only route to `VerifiedRegistration`: `Confirmed` when the observation is for the same pid and its `BirthStamp` equals the record's, `Ended` when the kernel proves the writer gone, `Unknown` otherwise.

`registered_runs` in `progress/registered_runs.rs` reads every sampled entry, requires the filename generation to match the record's, verifies against `birth_stamp::observe(pid)`, and retains one `CaptureDiagnostic` per problem. It maps `UnsupportedVersion` to `CaptureDiagnostic::UnsupportedRegistrationVersion { path, encountered, supported }` and `continue`s before inserting into `generations`; that record never becomes a `RegisteredRun`, never gains sweep authority, and neither it nor its log is unlinked. The capture assembly in `progress/capture.rs`, through `Capture::scan_root`, keys each reading by `CaptureKey { root, pid, incarnation, generation, birth }`, reads each named log once per scan, and pushes a `ConfirmedCapture` for every confirmed record. `sweep_ended` runs only for the reader's own uid directory and only for records verified `Ended`: it rereads the file, requires equality of the parsed registration with the scan's record, observes the kernel again, and only then returns `SweepDisposition::Remove(log)`.

### Birth stamps

`BirthStamp { boot, birth }` in `crates/cargo-tile/src/birth_stamp/mod.rs` is the comparison unit, and `observe(pid)` returns a `KernelObservation` bound to the pid it read. Linux (`birth_stamp/linux.rs`): the boot id file and field 22 of `/proc/<pid>/stat`; `NotFound` is `Ended`, any other error `Unknown`. macOS (`birth_stamp/macos.rs`): `libc::sysctl` with `KERN_PROC_PID`, decoding the `p_starttime` timeval at offset zero and truncating to whole seconds; boot from `sysctlbyname("kern.bootsessionuuid")`, validated as a UUID. An empty reply or `ESRCH` goes through `process_absence`, which reports `Ended` only when `kill(pid, 0)` returns `ESRCH`. Each platform caches its boot read, failure included, in a `OnceLock` for the process lifetime, and `boot_verification()` reports a cached failure to settings.

### From registration to row

Each scan (`census::scan` in `crates/cargo-tile/src/census/scan.rs`) takes `Capture::take(roots)` before the detail pass, adds every registered pid and its ancestors to the detailed set, then runs `include_registered_processes`, `identify_capture_wrappers` (processes whose argv forwards the registered command, the shim's JSON rewrite included, via `forwards_capture_command`), `collapse_shims` (an outer cargo with exactly one cargo child, same subcommand, same directory by `DirectoryComparison`), `identify_captures`, `select_rows`, `groups` and `associate_status`.

Ownership resolves through `Capture::select(pid)`: the lowest root index first, then within that root one confirmed key, otherwise `Ambiguous`. `Census::captured_run` walks parents up to `PARENT_WALK_LIMIT` for the nearest selection. `Census::direct_capture` yields `DirectAssociation::Direct` only when the walk from the process to the shim pid crosses nothing but observed wrappers; any other intervening process gives the row `CaptureMembership::Enclosing`. A directly captured process row takes its directory and command from the record per field (`row_fields`) before display formatting. A confirmed registration with no directly representing process row sources its own row through `registration_row`: the record's directory, `cargo <args>`, the shim pid, start from the registration file's mtime, cpu and managed `Unavailable(Unproven)`. `registration_directory` shortens the directory to `~` only when the record's `WriterHome` is known and equals the scanner's home, so another account's path is shown in full. `RowProvenance::{Uncaptured, Direct, Enclosing}` carries `CaptureContext { root, incarnation, account }`, and `GroupingIdentity` in `crates/cargo-tile/src/render.rs` groups on that plus the raw `WorkingDirectoryIdentity`, so headings carry `[account]` and rows from different accounts never merge.

### Process census and CPU accounting (`crates/cargo-tile/src/census/`)

**Module layout.** `mod.rs` re-exports only thirteen items used outside `census`: `command_name`, `DirectAssociation`, `SelectedProof`, `Measurement`, `InvocationId`, `VisibleParent`, `Ancestor`, `CargoGroup`, `CargoProcess`, `CompilerObservation`, `RowProvenance`, `RunStart`, `spawn_with_resolver`. Code inside `census` imports from the owning submodule.

| File | Contents |
| --- | --- |
| `process_identity.rs` | `InvocationId`, `RunId`, `ProcessIdentity`, `ProcessIdentities`, `CaptureMembership`, `VisibleParent` |
| `direct_capture.rs` | `DirectCapture`, `DirectAssociation`, `NearestRegistration`, `SelectedProof` |
| `invocation_cpu_accounting.rs` | Measurement and accounting types |
| `command_text.rs` | `CommandText`; `cargo_split` → `CargoArgvAbsence::{ArgvUnavailable, ProgramRejected}`, converted losslessly into `RowAbsence` and `SubcommandAbsence` |
| `scan.rs` | `Census` and every scan, attribution, row and grouping method; `CargoAncestry::{Owner, ParentUnavailable, WalkLimitReached}` |

**Measurements.**
- `Measurement<T>` is `Reading(T)` or `Unavailable(MeasurementAbsence::{FirstObservation, ReadFailed, Unproven})`. Adding any `Unavailable` to anything yields `Unavailable`.
- `InvocationMeasurements { compilers, cpu }` holds disjoint per-pid buckets. A nested cargo's bucket is its own and is never folded into its parent's.

**Cumulative CPU.**
- `InvocationCpuAccounting` persists across scans. It holds `owners`, `invocations: HashMap<InvocationId, InvocationCpuHistory>`, `targets`, `cache_owners`, `identities`, `observed`, `settled`, `reported`, and `publication`.
- `InvocationCpuHistory::measure(InvocationCpuContributions { tree, detached, nested }, evidence, now)`:
  1. Updates each `RetainedSubtreeCpuTime`. On Linux the subtree total is the maximum of the current sum. On other platforms each process's maximum observed time is retained after the process exits.
  2. On Linux, subtracts retained nested-cargo totals.
  3. Holds `accumulated` monotonic.
  4. Returns `Unavailable` when `evidence` is unavailable.
  5. Otherwise compares against the `InvocationCpuBaseline::{AwaitingFirstSample, Established { accumulated, at }}` from the previous scan. It returns `FirstObservation` for the first sample and `Unproven` for zero elapsed time or a lower counter.
- `Census::process_cpu_time` reads the per-process counter. Linux: `linux_cpu_time(pid)` reads `utime+stime+cutime+cstime` from `/proc/<pid>/stat`, `LinuxCpuSample::verify` binds the sample to the process lifetime through the field-22 birth stamp, and `linux_clock_ticks` reads `AT_CLKTCK` from `/proc/self/auxv`. Other platforms: sysinfo's `accumulated_cpu_time`, which excludes reaped children.
- Only the invocation's own counter is validated. A new or idle descendant never blanks the row.
- A parent that has accrued zero ticks of its own, and has no failed or unproven own sample, still publishes its descendants' combined CPU.

**Compiler-cache attribution.**
- A `rustc` outside any cargo ancestry is charged by `--out-dir` to the one invocation whose target directory contains it. The target directory comes from `--target-dir`, then `CARGO_TARGET_DIR`, then observed clients.
- `compile_owner` returns `CompileOwner::{Unique, Unknown, Ambiguous}`. `Ambiguous` charges nothing.
- `CpuAssignment::{Direct, Detached { owner, compiler }, Unassigned}`: ancestry wins over detached attribution.
- `CompilerCreditRetention::{Keep, Retire}` keeps a live compiler with its first owner.
- `Census::attribute(&details, smoothing, now)` takes the detailed `System` because target directories come from command lines.

**Smoothing.** `settle` moves each settled reading toward the latest sample over `CPU_SMOOTHING_SECONDS` and publishes every `CPU_REPORT_MILLIS`. An unavailable sample replaces the published reading at once.

**Grouping (`Census::groups`).**
1. Builds process rows and snapshots `process_rows: HashSet<InvocationId>`.
2. Only then appends registration-sourced rows (plus the `cfg(test)` `registration_rows`).
3. Assembles groups.
4. For each group, `subtree_cpu(&shares, &members, &process_rows)` walks the tree leaves first. It seeds a member from its pid bucket only when its identity is in `process_rows` and adds each descendant's pids once. Any unproven contributor or a cyclic parent chain makes the total `Unavailable(Unproven)`.

`CargoProcess.subtree_cpu` carries the result. `render::summary_rows` draws it for a promoted summary row. Command rows keep `aggregate_cpu`, which charges a pid shared by two rows once.

**Test adapters.** `groups_with_registration_rows_for_test` and `groups_with_cpu_for_test` are `cfg(test)`. `tests/summary_totals.rs` drives them.

### Account hooks and credentials (`crates/cargo-tile/src/hook.rs`, `cli.rs`)

- `HookOperation::{Install, Uninstall, Status}` drives one shared account protocol. `cli::all_accounts(operation)`:
  1. Requires root.
  2. Reads accounts through `hook::system_accounts()` (`getpwent`, sorted by name then uid).
  3. For `Install` only, runs `root_scan::prepare_shared_directory(CAPTURE_ROOT)`.
  4. Stages the running executable with `hook::stage_executable`: a root-owned `0755` copy under the sticky parent of `CAPTURE_ROOT`, removed with its directory on drop.
  5. Calls `hook::run_account_hooks(&accounts, staged, operation) -> Vec<AccountHookReport>`.
  6. Prints each report, then `N accounts: C completed, T with no toolchains, I incomplete`.
  7. Returns `operation.completion(&reports)`.
- `run_account_hooks` skips any account with no `<home>/.rustup`. As root it applies `account_credentials`; otherwise it uses `Command::uid/gid`. `run_account_hooks_with(accounts, executable, operation, impl FnMut(&mut Command, &HookAccount) -> io::Result<()>)` injects the credential step for tests. No root-owned file operation reaches an account's toolchain tree.
- The child runs `<staged> <subcommand> --account-hook-report` with `HOME` and `RUSTUP_HOME` set to the account's database home. It writes one `<toolchain>\t<result>` row per toolchain and flushes each row before the next toolchain starts.
- `ToolchainHookOutcome::{Install, Uninstall, Status(HookState), Unreadable, Failed}` encodes the rows. `AccountHookReport::from_output` decodes them and keeps valid rows past a malformed line and past a failed child exit.
- `AccountHookOutcome` is one of `NoToolchains`, `Completed` or `Incomplete(String)`. A credential failure becomes `Incomplete("could not resolve <name>'s groups: <error>")`, and later accounts still run.
- Local paths consume `Hook::reports(operation) -> io::Result<impl Iterator<Item = ToolchainHookReport>>` lazily through `.inspect(print_local_report)`.
- `Hook::at` returns `HookDiscovery::{Discovered, AbsentCargo, InspectionFailed { path, error }}`.
- `Hook::state` is the only inspection that runs without the lock. `Hook::install`/`ensure` and `Hook::remove` take `HookInstallationLock` (exclusive creation of `cargo-tile-shim.lock`, retried, removed on drop) before they classify `HookState::{Installed, Absent, Repairable, Orphaned}`.
  - `Repairable` means the real cargo was moved aside but the shim file is missing; install rewrites the shim without a second rename. `Orphaned` means a shim is present with no `cargo-tile-real` beside it; install reports it and does not overwrite.
  - `install` returns `HookOperationOutcome::{Installed, Refreshed, AlreadyCurrent, DowngradeRefused { installed, supported }, Orphaned}`.
  - `remove` returns `Removed`, `AlreadyAbsent` or `Orphaned`.
  - `is_incomplete` counts `Unreadable`, `Failed`, `Orphaned` and `DowngradeRefused`.
- `account_groups(name, primary_gid) -> io::Result<Vec<u32>>` wraps `getgrouplist` in the private generic `account_groups_with<Group: Copy>(primary_gid, resolve)`, resolving the complete membership in the parent. The loop:
  1. Resets the count to the buffer length before each call.
  2. Resizes to a reported larger count when one arrives. A zero, smaller or unchanged count doubles the buffer instead.
  3. Returns `InvalidData` for a negative count.
  4. Errors at `ACCOUNT_GROUPS_MAX_CAPACITY`, naming the ceiling.
- **Linux credentials:** `pre_exec` calls `setgroups(full list)`, then `setgid`, then `setuid`.
- **Darwin credentials:**
  - The parent builds `DarwinGroupMembership::new(uid, primary_gid, groups, sysconf(_SC_NGROUPS_MAX))` before fork. It:
    1. Requires a positive limit.
    2. Requires that the primary gid is in the resolved list, and swaps it to index 0.
    3. Sets `count = min(len, limit)` and converts uid to `c_int`.
  - The `pre_exec` child calls the raw libsyscall entry `__initgroups(count, groups.as_ptr(), uid)`, then `setgid`, then `setuid`. It does no allocation, no locking and no directory lookup. Apple's [Libinfo implementation](https://github.com/apple-oss-distributions/Libinfo/blob/39b70c515baee5b609e7e91693edbd934b6845a1/lookup.subproj/libinfo.c#L896) of `initgroups(3)` uses this entry after its directory lookup.
  - The kernel keeps the first `count` groups in the credential and resolves the remaining groups dynamically through the registered uid.
  - No account is refused and no membership is shortened.
- `unsafe_code` allowances in `hook.rs`, each with a `// SAFETY:` comment: `system_accounts`, `account_groups`, `account_credentials`.
- Shim versioning:
  - `shim_version(contents)` reads only the first `SHIM_MARKER_SEARCH_BYTES`. A declaration that does not terminate inside that window is `InvalidData("incomplete shim version line")`. So is a trailing fragment that is a proper prefix of `SHIM_VERSION_PREFIX`.
  - Under the lock, a newer declared version yields `DowngradeRefused` and leaves the shim, the saved real cargo, their modes and their mtimes untouched.
  - `capture::stand_up` reaches `hook::at_startup` when `auto_install` is true (the default), and folds `Startup.kept_newer: Vec<NewerShim>` into `App.capture_note: CaptureStartupNotice::{Quiet, InstallationFailed, NewerShimKept, NewerShimKeptWithFailures { kept, failures }}`. The startup toast and Settings render it.

### Terminal UI and settings scrolling (`crates/tui_pane/src/overlays/settings.rs`)

- `SettingsPane` holds a `Viewport`, `line_targets: Vec<SettingsLineTarget>`, `row_lines: BTreeMap<usize, Range<usize>>` and `drawn_scroll_offset: DrawnScrollOffset::{Undrawn, Drawn(usize)}`. `SettingsPane::new` is a `const fn`.
- `SettingsLineTarget` is one of `Row(SettingsRowPayload)`, `Decoration` or `OutsideContent`.
- `render_rows(&mut self, rows, options) -> SettingsRender` numbers selectable rows by position. Wrapped continuation lines carry their row's target.
- `update_scroll` keeps the selected row visible. `navigate` shows a tall row's hidden continuation lines before it moves to the next row.
- `render_lines(&mut self, frame, lines)` slices to the viewport, paints, and records `Drawn(offset)`.
- `row_at(pos) -> SettingsRowHit::{Row(usize), Missed}` resolves against the recorded drawn offset. Before the first paint every click misses. Keyboard navigation and mouse selection use the same rows.
- `line_for_selection(selection) -> SettingsSelectionLine::{Rendered(usize), NotRendered}`.
- `SettingsRowIdentity::{Decoration, Selectable(SettingsRowPayload)}` (`settings_store/row.rs`): a selectable payload must equal the row's zero-based position among selectable rows.
- Consumers: `cargo-tile` `render.rs` (geometry, `update_scroll`, then `render_lines`), `navigation.rs` and `interaction.rs`.
- Input: crossterm carries the `use-dev-tty` feature in `crates/cargo-tile/Cargo.toml`. This selects the level-triggered Unix input backend, so a keystroke that arrives together with a resize is not lost.
- Rows readout: `draw_with_readout`, `content_area` and `rows_readout_height` in `render.rs` split a tile's interior between contents and the readout row. The readout row counts toward height demand, and a one-row interior shows only the readout.

### Linux backdrop (`crates/tui_pane/src/backdrop/desktop/platform/linux/`, `backdrop/monitor/mod.rs`, `theme/poller.rs`)

**Shared subprocess reader (`mod.rs`).**
- `read_desktop_command(&mut Command) -> Result<Vec<u8>, DesktopReadFailure::{Expired, Failed(String)}>` spawns a `DesktopSubprocess`. That process's stdout is one end of a nonblocking `UnixStream::pair`, and stdin and stderr are null.
- It polls `try_wait` before each read of up to `DESKTOP_STDOUT_CHUNK_BYTES`, every `DESKTOP_READ_POLL_INTERVAL`, until `TOPOLOGY_READ_DEADLINE`.
- On expiry or error it kills and reaps the child even when the kill fails. A nonzero exit is `Failed`.
- `kscreen-doctor` and `kdotool` both go through this reader.

**Session bus.**
- `SESSION_CONNECTION: Mutex<SessionConnection<Connection>>` is one of `Unconnected`, `Connected` or `Retrying { failure, retry_at }`.
- `session_connection() -> SessionBus::{Connected(Connection), Unavailable(ConnectionFailure::{NeverConnected, Disconnected, StatePoisoned})}` clones the held connection. It reconnects only after `retry_at` (`DESKTOP_RETRY_INTERVAL`) or when `is_closed`.
- Capture, position and topology workers all share this one connection.

**Display topology (`display.rs`).**
- `TOPOLOGY: Mutex<DisplayTopology::{Unread, Watching(Vec<Output>), Recovering(TopologyRead)}>`. `TopologyRead` is `Read(Vec<Output>)`, which may be empty, or `Unreadable`.
- `active_outputs()` starts one watcher thread through a `Once` and returns the snapshot. Capture ticks never spawn `kscreen-doctor`.
- `watch_connected_topology`:
  1. Subscribes to KScreen `configChanged` and `NameOwnerChanged` for `org.kde.KScreen`, each buffered to `TOPOLOGY_SIGNAL_CAPACITY`.
  2. Calls `requestBackend` and records the replying owner.
  3. Reads the layout (`kscreen-doctor -j`).
  4. Re-reads on each change.
  5. Ignores an owner-change signal naming the owner that answered.
- On interruption the state becomes `Recovering(last snapshot)`, and the watch retries after `DESKTOP_RETRY_INTERVAL`.
- `parse_outputs` keeps connected, enabled outputs with a finite positive scale and swaps size for rotation 2 or 8.
- `under(outputs, frame) -> OutputSelection::{Containing, Nearest, NoActiveOutputs}` picks by window centre.
- `capture` reports `Unread`, `Unreadable` and an empty layout as `CaptureFailure::DisplayNotFound`.

**Terminal window inventory (`window.rs`).**
- `TERMINAL_WINDOWS: LazyLock<Mutex<TerminalWindowInventory>>` has three parts:
  - `search: TerminalWindowSearch::{NotSearched, Found { uuids, searched_at }, Unavailable { failure, tried_at }}`.
  - `registry: WindowRegistry`: a UUID↔`u32` handle map whose handles stay stable across refreshes.
  - `classifications: HashMap<String, TerminalClassification::{Unclassified, Known(TerminalClass::{Matching, Other})}>`.
- The one discovery command is `kdotool search --name .* getwindowid %@`, serialized by `KDO_TOOL_ACCESS` because concurrent `kdotool` runs collide on their temporary KWin scripts.
- `read(usage, now, source)` for `WindowInventoryUse::{Capture(target), Identification, MarkerIdentification}`:
  1. A `Capture(PreferWindow { window_id })` whose registered window still answers returns that window with no discovery.
  2. Otherwise it queries held UUIDs through `held_windows`.
  3. It searches again only when nothing has been searched, or when the last search or failure is at least `DESKTOP_RETRY_INTERVAL` old and either no windows were found or the use is `Capture(TerminalWindowHeuristic)`.
- `held_windows`:
  - Skips querying `Known(Other)` UUIDs unless the use is `MarkerIdentification`.
  - Sends one `getWindowInfo` per remaining UUID over the held bus.
  - Drops a UUID and its classification when it does not answer.
  - Filters the result to `TerminalClass::Matching` except for marker identification.
- `TerminalClass` is `Matching` when the KWin `resourceClass` contains `TERM_PROGRAM`, ignoring ASCII case.
- `frame(handle)` queries one registered UUID and drops it on no answer.

**Position worker (`monitor/mod.rs`).**
- `PositionReadState::{Unwatched, Holding { window, position, read_at }}` with `WindowPosition::{Unavailable, Settled(Frame), Moving { frame, unchanged_since }}`.
- `read` returns the held answer without querying when the same window is `Settled` or `Unavailable` and less than `POSITION_IDLE_INTERVAL` has passed since `read_at`. Otherwise it reads.
- `observe` enters `Moving` on a changed frame. The state stays `Moving` while the frame has been unchanged for less than `POSITION_SETTLE_DURATION`, then becomes `Settled`.
- A different window reads at once.
- `position_loop_with(watches, frames, clock, read_frame)` sends one `Option<Frame>` per request.

**Appearance (`theme/poller.rs`, Linux).**
- `AppearanceBackend` (`connect`, `watch`, `read`) is implemented by `PortalBackend` over `org.freedesktop.portal.Settings`.
- `subscribe` connects, then installs the `SettingChanged` (`org.freedesktop.appearance`/`color-scheme`) and owner-changed streams, then reads the startup value, all on one connection.
- `follow(initial, stream, history, recovery, on_change: FnMut)` sets `SubscriptionRecovery::Live`.
- `AppearanceHistory::{Unobserved, Observed(AppearanceObservation::{Unspecified, Selected})}` delivers only a changed `Selected` value.
- The stream ends with `SubscriptionEnd::{Disconnected, OwnerChanged, InvalidSetting}`.
- `track` logs `tracing::warn!` on each end or failure. `SubscriptionRecovery::{NeverConnected { failures }, Live, Interrupted { failures }}::failed()` waits `POLL_INTERVAL` until `BACKOFF_THRESHOLD` failures, then `BACKOFF_INTERVAL`.
- Non-Linux keeps the polled `dark_light::detect`.
- Manifests: workspace `futures-lite = "2.6.1"` and `zbus = "5.19.0"`. Both are non-optional Linux dependencies of `tui_pane` and are not tied to `backdrop`.

### cargo-berth reader compatibility (`crates/cargo-berth/`)

- `board --reservation <id> --json` reports `Race extent` and `Merge extent` in `ReservationReport` (`board/report.rs`). An unresolvable trunk yields `unavailable` with the prior extent as `retained_evidence`.
  - The success variant is `Snapshot(Box<ReservationReportSnapshot>)`, which is untagged, so the JSON shape does not change.
  - `docs/cargo-berth/generated/output-contract.json` carries the extent fields.
- `reconcile.rs`: `prepare_reconciliation_transaction` calls `derive_merge_extents` first and then `append_evidence_operations`. `prepare_gate_reconciliation` uses the same appender.
  - Every persisted evidence record derives `edit_blocking_status` through `Reservation::edit_blocking_status` (`reservation/record.rs`) on the post-transaction reservation from `with_merge_extent`.
  - `append_evidence_operations` takes out the scoped-patch scheduling operations, appends evidence, and puts them back, so retry priorities survive.
- `IntegrationEvidenceStatus::edit_blocking_status` (`reservation/lifecycle.rs`) is `#[cfg(test)]` and is kept only for historical replay comparison.
- `verb/release.rs` `outstanding_operation`: on an unreadable trunk it persists `ObjectUnknown` through `evidence_operation`. The retained merge extent's protection then decides clear versus blocking.
- Historical records replay byte-for-byte unchanged.
- `tests/reader_compat.rs` checks a chosen reader:
  - It runs the executable named by `CARGO_BERTH_EXECUTABLE` against a frozen ledger restored from `tests/fixtures/reader_compat/repository.bundle`.
  - Five cases: `board`, `check` and `drift` responses, plus SessionStart and PostToolUse hook payloads.
  - It compares the canonicalized root against expectations built in `tests/support/reader_compat_hooks.rs` (shared with `tests/hooks.rs`).

### Runner services and upgrade procedure

**Services.**
- Linux: `github-runner-hana-linux-1.service` (uid 992) and `github-runner-hana-linux-2.service` (uid 991). The units are generated by the machine's NixOS configuration. Units use `UMask=0066` and `PrivateTmp=yes` with `BindPaths=/tmp/cargo-tile`. Runtime `HOME` is `/run/github-runner/<account>`, state is `/var/lib/github-runner/<account>`, `RUSTUP_HOME` is `/var/lib/hana-ci/<account>/rustup`, and `CARGO_HOME` is `/var/lib/hana-ci/<account>/cargo`. The generated unconfigure `ExecStartPre` deletes the runtime HOME contents, so a config placed there does not survive a restart.
- Mac: `org.nixos.hana-macos-runner` runs as `hana-ci` (uid 502). The plist is `/Library/LaunchDaemons/org.nixos.hana-macos-runner.plist`, with working directory `/Users/hana-ci/actions-runner`; its Nix launcher rewrites `.env` and `.path` at startup. The plist sets neither `CARGO_TILE_ROOT` (read by nothing) nor a job-start hook.
- The three runners are registered to the `hana` repository. cargo-liner CI uses GitHub-hosted runners, so runner rows are observed from hana CI runs.

**Upgrade order.** Readers are deployed before v3 writers, and shims only after every reader on the machine is upgraded:
1. Before any upgraded reader starts, set `capture.auto_install = false` for every account (keeping each original config's bytes and mode, or its absence) and withhold runner job-start hooks. On Linux the runner accounts need a NixOS configuration change and a rebuild first, because the unconfigure pre-start wipes runtime HOME: a last-position `ExecStartPre`, run as the runner account, copies the suppressed config into runtime HOME.
2. Deploy the readers (`cargo-tile`, `cargo-berth`, `cargo-port`) and restart them.
3. Run `sudo cargo tile install --all-accounts`.
4. Verify each shim's version line.
5. Run `status --all-accounts` and check that every expected toolchain reports `Installed`.
6. Restore each account's original config bytes, mode, or absence.

### What the tests prove

`crates/cargo-tile/tests/shim_registration.rs` reaches the sources through `#[path]` modules and drives the built binary under a PTY. `crates/cargo-tile/tests/support/shared_capture.rs` holds the cross-account scenarios. Fixtures write records with Python under the test's own uid; no test needs a second account.

- Injected accounts install through the built binary. The staged installer copies the executable and removes its directory on drop. An installer that cannot start is reported per account, not as a crash. Group resolution matches the account database. Unit tests cover the Darwin primary-first list, credential prefix and uid, and continuation past a credential failure.
- The reader reports a foreign-owned uid directory as ignored, skips symlinked and non-numeric entries, and shows a new account directory on the next scan.
- The reader creates the parent when missing and repairs a mode it owns.
- A live capture under the real path, scanned through the `/private/tmp` alias, survives the sweep. A record carrying a legacy `{ sec = ` boot is reported `IdentityUnknown` and never unlinked.
- The reader reaps its own ended capture and leaves a foreign-owned directory untouched. The obsolete `roots` key is ignored.
- `reader_regression` covers nested shims, a quiet JSON caller, recovery from ambiguity, locale and timezone variation, legacy registration survival, and removal of an ended staging file.
- `tests/summary_totals.rs` covers promoted-summary CPU totals, including zero-own-tick parents and unreadable counters.
- `tests/capture_root_acl.rs` (`#![cfg(target_os = "macos")]`, 20 tests) exercises real ACLs through `/bin/chmod +a` with `CAPTURE_ACL_TEST_*` fixture constants.
- cargo-berth `tests/reader_compat.rs` checks a chosen reader against the frozen `merge_extent_observed` ledger.

## Invariants

- The shim never alters what cargo does, prints, or exits with. Every setup failure degrades to `exec "$real" "$@"`.
- The shim is POSIX `sh`: no bashisms, no `readlink -f`, no `local`.
- Publication order and sampling order are one invariant read from two ends. The shim publishes the registration before it creates the log. The reader samples registrations when it opens the root and collects the sample before consulting liveness.
- A sweep removes `<pid>.<generation>` and the log the record names as a pair, decided by fresh kernel evidence for that pid immediately before the unlink, never by age or by name alone. `Unknown` authorizes neither a row nor a deletion.
- Sweep authority comes from `access` alone. Each file needs its own descriptor proof and a dev/ino recheck immediately before `unlinkat`. The log is unlinked before its registration. Directory write grants and incomplete inventories never veto a proved pair.
- Only the reader's own uid directory is swept (`CaptureCleanup::Here`). A foreign-owned directory is reported, not read. Another account's dead captures are reaped by that account's next shim invocation.
- No sweep count or refusal reaches Settings or any user surface.
- `VerifiedRegistration` is the only proof of identity, and production `KernelObservation` values come only from `observe(pid)`.
- A registration newer than `SUPPORTED_REGISTRATION_VERSION` never becomes a `RegisteredRun`. The version is classified from the bounded header before size or fields.
- Every directory is opened with `O_NOFOLLOW` relative to a held handle. Ancestor symlinks are resolved once by `canonical_capture_path`; the final component never is.
- `install` exits zero when a toolchain fails, because job-start hooks call it. `uninstall` and `status` exit nonzero when any account is incomplete.
- `Hook::reports` is lazy. Each consumer prints or forwards an item before it pulls the next. A child flushes each row before starting the next toolchain.
- A mutating hook operation takes `HookInstallationLock` before inspecting state. `Hook::state` is the only inspection without the lock.
- A reader never replaces a shim that declares a newer version.
- A supplementary-group list is never shortened.
  - Darwin: pass the credential-limit prefix and the uid to `__initgroups`, with the primary gid first.
  - Linux: pass the full list to `setgroups`.
  - All allocation and conversion happen in the parent. `pre_exec` makes credential syscalls only.
- `Census::groups` snapshots `process_rows` before it injects registration rows. Subtree totals seed only members in that set. CPU aggregation keys on invocation identity and charges a pid once per lead.
- Detached compiler work is charged only for a `CompileOwner::Unique`. Ancestry takes precedence.
- `progress/mod.rs` and `census/mod.rs` re-export only items used across the crate. Other consumers import from the owning submodule.
- Settings content is painted through `SettingsPane::render_lines`. A selectable row's payload is its position among selectable rows. The settings pane does no filesystem access; everything it shows was observed on the worker.
- No capture tick spawns `kscreen-doctor`. `kdotool` discovery runs at most once per `DESKTOP_RETRY_INTERVAL` except on the startup or no-window path. One process-wide session-bus connection serves every desktop worker.
- Every desktop subprocess goes through `read_desktop_command`, and every expiry or failure kills and reaps the child. Every `kdotool` invocation holds `KDO_TOOL_ACCESS`.
- cargo-berth evidence records are built after extent derivation and match the live `Reservation::edit_blocking_status`. Historical journal records are never rewritten.
- Every cargo-berth reader that can reconcile or release includes the extent and evidence handling before a newer binary touches a shared ledger.
- `unsafe_code` is denied workspace-wide. In cargo-tile the exceptions are `birth_stamp/macos.rs` (sysctl), the account-database and credential calls in `hook.rs`, and the test FIFO fixture in `root_scan/inspected_directory.rs`, each under a per-item `allow(reason)` with a `// SAFETY:` comment.
- Every constant lives in `crates/cargo-tile/src/constants.rs` with a rationale.
- The crate is binary-only. Tests of crate items are inline `#[cfg(test)]` modules. Each integration harness that reaches the sources (`shim_registration.rs`, `summary_totals.rs`, `capture_root_acl.rs`) has its own full `#[path = "../src/…"]` block; moving a file means updating every block.
- Two uids are never required by a test.

## Calibration and gotchas

**Capture, registration and sweeps.**
- `CAPTURE_SWEEP_LIMIT` is 512 removal attempts per scan across all roots; a pair costs two, reserved before the first unlink. `CAPTURE_INVENTORY_LIMIT` is 4096 entries, `CAPTURE_ENTRY_NAME_BYTES` 255, `CAPTURE_REGISTRATION_BYTES` and `RUN_LOG_TAIL_BYTES` 64 KiB, `BIRTH_SYSCTL_MAX_BYTES` 4096. `REGISTRATION_VERSION_HEADER_BYTES` is `len("cargo-tile-v") + 20`. The worker sleeps `PROCESS_POLL_MILLIS` (250) after each scan, so the scan rate is at most four per second.
- When a live run's log is swept, cargo reopens it through `script` or `tee -a` outside the setup subshell under the caller's umask (`0066` on the runner units), so it lands `0600` and stays unreadable for the rest of that run. That is the reason for the ordering invariant.
- `read_dir` is lazy; collecting is the sample. Keeping the calls in order but dropping the collection reintroduces the defect.
- An unreadable registration retains its path-qualified Settings diagnostic and is preserved, while independently proved ended sibling pairs remain eligible for sweeping.
- The final identity check and `unlinkat` are separate syscalls. A basename replaced between them can be unlinked. Retained handles prevent traversal redirection, not that final replacement; no POSIX conditional unlink exists, so the recheck detects replacement and does not guarantee a replaced file survives.
- A mode or group change on a capture directory between scans does not count as a changed directory. `same_directory` compares dev, ino and owner only.
- `effective_user()` and each platform's boot read cache their first result, failure included, for the process lifetime. `IdentityUnknown` (retried next scan) and `IdentityBlockedByBoot` (restart required) stay separate for that reason.
- macOS: `kern.boottime` is recomputed on clock corrections, so the boot field is the session UUID. `BirthStamp::compare` returns `Unknown`, never `Ended`, for a legacy `{ sec = ` boot string against a present process. The macOS birth truncates to whole seconds; the generation, not the birth, separates two invocations born in one second. `ps -o lstart=` is padded to column width and rendered through `localtime`, so the shim trims it and reads under `LC_ALL=C TZ=UTC0`. `/tmp` is `/private/tmp`; `DirectoryComparison::between` compares aliased directories by device and inode, and relative paths never establish identity.
- A runner service whose PATH lists Nix or Homebrew coreutils ahead of `/bin` resolves `date` to the GNU binary, which rejects `-j`. The shim tries the BSD form and falls back to `date -u -d`, so the birth field is filled under either binary. An empty birth field means the tile can neither draw nor delete that registration.
- The state column draws `blocked` only for `waiting for file lock on build directory`; a package-cache lock wait is ignored by design.

**Hooks and installation.**
- `ACCOUNT_GROUPS_INITIAL_CAPACITY` is 32, `ACCOUNT_GROUPS_GROWTH_FACTOR` is 2, and `ACCOUNT_GROUPS_MAX_CAPACITY` is 65,536 entries (256 KiB).
- glibc writes the required count back through the count pointer. Darwin can leave the count unchanged, which is why the loop resets the count and grows.
- Darwin `_SC_NGROUPS_MAX` is 16. macOS adds implicit groups (12, 61, 701, 100), so an account with 17 explicit groups resolves to about 21.
- `SHIM_MARKER_SEARCH_BYTES` is 1024 and the version declaration sits near byte 44. A comment `#` landing at the window boundary would make install refuse a correct shim.
- `status --all-accounts` checks installation state, not framing or shim bytes. Only each shim's version line proves v3. A zero status exit can accompany `Absent` or `Repairable` rows, so every expected toolchain must actually report `Installed`. Top-level setup or discovery can still fail before per-account report handling.
- Neither reader version strings nor shim version lines identify which reader build is installed; compare executable hashes.
- Restarting a reader installs shims unless `capture.auto_install` is false. A TOML type error anywhere falls back to defaults, which set `auto_install = true`.
- The local install path ends `.inspect(print_local_report).count()`. It only iterates because a `filter_map` chain is not `ExactSizeIterator`.
- A cargo-tile older than v3 framing writes `cargo-tile-v2`. When install progress is missing, check the installed version first.
- The staged executable relies on the parent of `CAPTURE_ROOT` being sticky and root-owned.

**CPU and census.**
- Two invocation identities can share one pid: a registration fallback and a process row whose command differs. Anything keyed on pid charges that pid twice.
- A contested compile is refused silently: the row shows only its own tree's time.
- On non-Linux platforms the counter is sysinfo's: `accumulated_cpu_time()` is quantized, and a rate is trustworthy only when the previous accumulated counter is positive. Start times are whole seconds, which is why registration-sourced rows use the registration mtime. `UpdateKind::OnlyIfNotSet` never re-reads a populated field, so the detail pass builds a fresh `System` per scan.
- An import used only under `cfg(target_os = "linux")` passes Linux lint but fails macOS lint as unused. `invocation_cpu_accounting.rs` guards those imports.
- `RowAbsence::ProgramRejected` and `PolicyExcluded` both render as `GroupAbsence::Excluded`.

**Settings.**
- `HashMap::new` is not `const`, so `row_lines` is a `BTreeMap` to keep `SettingsPane::new` a `const fn`.
- Crossterm's default edge-triggered backend drops a readiness token when a resize arrives in the same batch. Only `use-dev-tty` prevents this.

**Backdrop.**
- `DESKTOP_RETRY_INTERVAL` is 30 s, `TOPOLOGY_READ_DEADLINE` is 5 s (it bounds `kdotool` too), `DESKTOP_READ_POLL_INTERVAL` is 10 ms, and `DESKTOP_STDOUT_CHUNK_BYTES` is 8192. `CAPTURE_REFRESH` stays at 1000 ms.
- `POSITION_IDLE_INTERVAL` is 250 ms and `POSITION_SETTLE_DURATION` is 500 ms. A move can register up to one idle interval late.
- A plain `kdotool getwindowid` returns only the first match. `%@` returns every UUID.
- `.*` holds windows whose class does not match `TERM_PROGRAM`, so marker titles still resolve.
- The first capture tick after startup can land on `Unread` and report `DisplayNotFound` once.
- Backdrop capture runs only while the attract screen is visible.
- `backdrop` is off by default in `tui_pane`, so topology, selection and bus tests run only with that feature.
- A `NameOwnerChanged` emitted at `AppearanceBackend::watch` would cause a reconnect loop. This has not been observed.
- Measured on a KDE Plasma machine over one idle minute: 2 `kdotool` children, 0 `kscreen-doctor` runs, 0 new session-bus connections, and `getWindowInfo` at about 4.5/s.

**cargo-berth.**
- With `CARGO_BERTH_EXECUTABLE` unset, `reader_compat` tests the freshly built binary and passes silently.
- Passing `reader_compat` does not prove a reader has the evidence handling. The fixture exercises neither `board --reservation` nor an evidence transition.
- An evidence record describes its own moment, so compare against the live answer.
- `release::execute` returns the reconciliation result before reaching `outstanding_operation`. A test must materialize `object_unknown` with a board read first.
- A fixture must commit its berth configuration before creating worktrees. An untracked config adds merge protection.

**Platform and tests.**
- XNU hides the platform `/bin/bash` environment even from root. The runner Listener process carries the runner environment instead.
- An unsigned copy of `/bin/sh` or `/bin/bash` is killed on macOS, and `/bin/sh` re-execs as bash. Darwin fixtures ad-hoc sign a copied `/bin/bash`.
- Darwin fixtures canonicalize the `/var` alias and set `PYTHONUTF8=1`.
- On Linux, the cache-readiness check retries an unavailable `ps` within its deadline.
- Fixture writers live about 20 s. The two 10 s `wait_for` deadlines plus the 1 s settle already use about 21 s, so a new wait replaces an existing one. `wait_for_fixture_pane(markers)` is the readiness predicate.
- `capture_root_acl.rs` compiles only on macOS. A green Linux gate says nothing about it.
- An empty ACL (a live allocation with zero entries, from `chmod -a# 0`) differs from an absent one (`-N`).
- `reader_keeps_application_spawned_cargo_when_run_is_excluded` is timing-sensitive: about one flake in three package runs, green on rerun.

## Why

**Shared directory and sweeps.**
- One shared parent with per-uid children, not a configured root list: the shim, reader and installer all know one machine location, and a runner account needs no configuration. Sticky `1777` lets any account create its directory while only the owner can replace it.
- Cleanup is per account because a non-owning reader cannot unlink under a `0755` directory, and every cross-uid removal attempt would fail on every entry, permanently. The shim reaps its own predecessors instead.
- One ownership rule on both platforms, not a platform gate returning foreign for every non-Linux root, under which no Mac would ever sweep.
- Each file is proved on its own, not through the directory, because a world-writable or ACL-granted directory says nothing about who owns a given file. Once each file carries its own proof, no ACL inspection and no cleanup reporting is needed.
- `SweepAdmissionRefusal` does not include `EnumerationIncomplete` because an incomplete inventory is counted inside an already-admitted sweep.
- Canonicalising the whole account path was ruled out; it would resolve the final component and defeat the `O_NOFOLLOW` check.
- Recursive mode correction beneath the root was ruled out; one level, only directories the shim owns. Changing the runner unit's `UMask=0066` was ruled out; it reaches every file the runner writes, credentials included.

**Registration and identity.**
- Hard-link publication, not `mv -f`: a rename silently replaces a name a live run still owns, while `ln` fails and a failure is a setup failure like any other.
- NUL framing, not a `<cwd>\tcargo <args>` line: space-joined argv loses argument boundaries, and a tab or newline inside an argument breaks the framing.
- `/proc/$$/stat`, not `/proc/self/stat`: inside the setup subshell `self` is the short-lived child, not the pid the registration is filed under.
- Verification by kernel evidence before deletion: exclusive creation prevents a name being taken while a record holds it, and does not prevent the same name being published again once removed, so deleting by exact name is not safe on its own. Age cannot establish that a paused writer ended, so a staging file is eligible only when the kernel confirms its writer gone.
- The version is classified before size so a newer oversized record is reported as skew, not as unreadable. Parsing a truncated version declaration from its visible prefix was ruled out: it reads a newer shim as older and overwrites it.
- `libc::sysctl` on macOS, not `sysctl(8)` or `ps(1)`: a subprocess per registration per scan at four scans a second. rustix has no sysctl binding, and sysinfo's start times are rounded on Linux and zero on the macOS fallback.
- The boot session UUID, not `kern.boottime`: the latter follows clock corrections, so every live registration mismatches and is swept within a scan.
- `kill(pid, 0)` before calling a process ended: an empty `KERN_PROC_PID` reply can mean a hidden record, not an exited process.
- `geteuid` from `rustix::process`, not a process-table view, which depends on process visibility.
- Registration mtime as the run start: it is the one value the reader already has across a uid boundary.
- A second row for a pid confirmed under a non-selected root was ruled out; one live invocation is one tile, and `Ambiguous` means no owner, not pick one. Letting `CaptureKey`'s derived `Ord` decide ownership was ruled out; a filename string would pick the live generation among equally confirmed registrations.
- Resolving the owner uid to a name at render time was ruled out; the worker resolves it and grouping keys on the number.
- Capturing `cargo tile`, `cargo port` and `cargo berth` was ruled out; the first two would run a terminal UI under `script`, and the third fires several times a second with no build to mirror.

**Hooks and credentials.**
- A staged root-owned installer copy under `/tmp`, not the administrator's own binary: the account child must execute it whatever the permissions on the administrator's home, and a sticky parent stops another account replacing it.
- Credentials applied together in `pre_exec`, group credentials first: `Command::uid` alone leaves the child without the account's supplementary groups, so a toolchain the account reaches through a group would fail.
- `__initgroups` instead of `setgroups` on Darwin: `setgroups` caps membership at the credential limit and removes dynamic resolution, so every group past 16 is lost. Refusing or truncating over-limit accounts was ruled out because a supplementary list is the account's whole membership to the kernel. A 21-group disposable account confirmed the mechanism: the switched child reads a file through a group past the first 16, and a child given the identical 16-group prefix through `setgroups` gets `EACCES`; `status --all-accounts` completes for that account.
- The primary gid is placed first because the `setgid` that follows must not displace a group in the credential.
- Groups are resolved in the parent because a forked child of a threaded process can only make async-signal-safe calls.
- One resize loop serves both libcs, so no `cfg` branch is needed for the count contract. An unbounded retry was ruled out because it would spin forever on a libc that never reports a size.
- Rows stream one toolchain at a time so an interrupted child still reports finished work. Collecting all reports before printing was ruled out.
- `install` exits zero and `uninstall` exits nonzero because job-start hooks must not fail over capture setup, while automation must not read a partial uninstall as complete.
- Restoring `cargo-tile-real` before installing over it was ruled out; the round trip is the window in which a second installer saves a shim as the real cargo. A file-locking dependency was ruled out in favour of the existing exclusive-create idiom.

**CPU and census.**
- Counters are cumulative, including `cutime`/`cstime`, because rates from per-process snapshots miss reaped children and blank the row whenever a descendant appears or exits.
- A contested compile charges nothing because every other choice overstates some row.
- Command-text parsing is not folded into `scan.rs`: `scan.rs` is the largest file, and parsing has its own tests and no `Census` dependency.
- Types are not re-exported beyond cross-crate use, so each consumer names the owning module.

**Settings.**
- Hit testing uses the drawn offset because a click in the same input batch as a navigation key must select the row that is on screen.
- Wrapping the mutable viewport accessor was ruled out because it breaks `const` callers.
- A crossterm workaround in the application was ruled out because crossterm's own first readiness check can drop the key before any handler is installed.

**Backdrop.**
- Topology is watched, the bus connection is held, and appearance is subscribed because polling each of them opened about a hundred session-bus connections a minute across the binaries: 60 `kscreen-doctor` spawns and 40 `Settings.ReadOne` calls.
- Subscribing before reading keeps a change that lands during the startup read.
- One class-independent `kdotool` search is shared by capture and marker identification because class-scoped discovery misses windows whose class does not match `TERM_PROGRAM`.
- Querying every desktop window on each capture tick and reading position every render frame were ruled out; together they drive `getWindowInfo` to about 27/s.
- `dark-light` has no subscribe entry point. `follow` takes its callback by value because a shared reference held across an await needs `Sync` on the public bound.

**cargo-berth.**
- Evidence is appended after extent derivation so each record describes the extent the transaction persists, not the previous one. A derivation from the checkpoint alone reports `Clear` without looking at the merge extent.
- Editing existing ledgers to remove `merge_extent_observed` was ruled out; older readers are upgraded instead.
- `reader_compat` uses a frozen bundle so every reader faces the same historical journal, without touching a shared ledger.
