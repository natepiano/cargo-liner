# cargo-berth — worktree coordination

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Builds `cargo-berth`, a git-worktree reservation engine, in the `cargo-liner` workspace, and wires it into the `hana` repo's Claude Code environment. This plan lives in `cargo-liner` because phases 1–11 build here; phases 12–17 run in `/Users/natemccoy/rust/hana`.

> **As-built disposition: create**

The design this builds is **in this file**, under `## Design` below, together with the
69 review findings (R1–R69) and eight resolved decisions (D1–D8) that shaped it. Work
Orders cite it by section heading and finding id — everything a delegate needs is here.

## Delegation Context

- **Project:**
- **Project started:** 2026-08-23T13:34:54-04:00
  - **Track A (engine, phases 1–11):** `cargo-berth` — new binary crate `crates/cargo-berth` in `/Users/natemccoy/rust/cargo-liner` (workspace members `cargo-mend`, `cargo-port`, `cargo-tile`, `tui_pane`). hana-blind. Publishes to crates.io.
  - **Track B (wiring, phases 12–17):** `/Users/natemccoy/rust/hana` — Claude Code integration. Almost no Rust.
- **Stack:** Rust edition 2024, resolver 3. Workspace-inherited `[workspace.package]` and `[lints] workspace = true`. Deps used, all as `{ workspace = true }`: `clap 4.6.6` (derive), `serde 1` (derive), `serde_json 1`, `toml 1.1.4`, `anyhow 1` (binary), `thiserror 2` (error enums), `cargo_metadata 0.23.1` (tier 2, deferred), `ratatui 0.30.2`, `crossterm 0.29.0`, `tui_pane` (path), `chrono 0.4.45`, `uuid 1` (features `v7`, `serde`), `tempfile 3.27.0` (dev). Git access is `std::process::Command`, no git library. File locking is `std::fs::File::lock` — **no new dependency**. `uuid` **is** a new dependency, added by phase 2: R39/R51 require opaque non-recyclable UUID-v7 identity and std has no random source, so it cannot be avoided. Add it to the root `[workspace.dependencies]` before the crate inherits it.
- **Layout:**
  ```
  /Users/natemccoy/rust/cargo-liner/            # Track A
    Cargo.toml                                  # members = ["crates/*"] is a glob — a new dir needs NO root edit
    README.md                                   # "## workspace members" gains a cargo-berth row
    rustfmt.toml  taplo.toml
    .claude/config/release.toml                 # single-package release cadence
    crates/cargo-berth/                         # NEW
    crates/cargo-tile/                          # the pattern to copy
    crates/cargo-port/src/project/git/          # the established git-subprocess pattern
    crates/tui_pane/src/lib.rs                  # board TUI framework

  /Users/natemccoy/rust/hana/                   # Track B
    .claude/config/berth.toml                   # NEW, beside release.toml + mirror.toml
    .claude/settings.local.json                 # has NO "hooks" key — track B creates it
    docs/hana/tool-graph.md                      # 19 todo phases / 20 Work Orders
    docs/hana_valence/arrangements.md            # 9 todo phases / 9 Work Orders
  ```
- **Key files:**
  - `Cargo.toml` — workspace manifest. `members = ["crates/*"]` is a glob, so creating `crates/cargo-berth/` registers the member with no root edit. `[workspace.lints.clippy]` and `[workspace.lints.rust]` live here; members inherit via `[lints] workspace = true`.
  - `crates/cargo-tile/Cargo.toml` — the manifest pattern to copy: inherited `authors`/`edition`/`license`/`repository`, explicit `name` + `version = "0.1.0-dev"`, `categories`, `keywords`, `homepage = ".../tree/main/crates/cargo-berth"`, `readme`, `[lints] workspace = true`, every dep `{ workspace = true }`, `tempfile` under `[dev-dependencies]`.
  - `crates/cargo-tile/src/main.rs` — binary pattern: crate-level `//!` doc (required, `missing_docs` is denied), flat `mod` list, `fn main() -> ExitCode { cli::Cli::parse_arguments().run() }`. No `[[bin]]` section — the binary name is the package name via `src/main.rs`.
  - `crates/cargo-tile/src/cli.rs` — clap `Parser`/`Subcommand` pattern including the `cargo berth <verb>` vs `cargo-berth <verb>` dual spelling: `parse_arguments` swallows the extra word cargo injects.
  - `crates/cargo-port/src/project/git/command.rs` — the git-subprocess pattern to follow: `git_command(repo_root) -> Command` with `--no-optional-locks` and `.current_dir()`, and `git_output_logged(repo_root, op, args)` wrapping it with timing/`tracing::trace!`.
  - `crates/cargo-port/src/project/git/constants.rs` — every git binary name, subcommand, flag, and ref prefix is a named `pub(super) const`. Follow this; never inline a git string literal.
  - `crates/cargo-port/src/project/git/worktree_group.rs` — existing worktree-grouping code.
  - `crates/tui_pane/src/lib.rs` — board TUI foundation, flat re-exports at the crate root. Entry types: `AppContext` (the trait an app implements), `Framework`, `PaneRegistry`, `Renderable`, `Pane`/`PaneFrame`/`FocusedPane`/`PaneChrome`, `Keymap`/`KeymapBuilder`/`Bindings`/`Action`/`KeyOutcome`, `PaneGridLayout`/`Region`/`Viewport`, `StatusBar`/`StatusLine`, `Theme`/`ThemeRegistry`, `Toasts`, `SettingsStore`.
  - `README.md` — member row shape: `- [name](crates/name) — description [![crates.io](https://img.shields.io/crates/v/NAME.svg)](https://crates.io/crates/NAME)`.
  - `.claude/config/release.toml` — single-package cadence: `/release <crate> X.Y.Z`, deliberately no `workspace_publish`. A path-only dep needs a `[[publish_path_pins]]` entry.
  - `/Users/natemccoy/rust/hana/.claude/config/release.toml`, `mirror.toml` — repo-scoped tool config precedent: plain TOML under `.claude/config/`, opening comment block explaining the tool and this repo's dialect, per-repo policy only.
  - `/Users/natemccoy/rust/hana/.claude/settings.local.json` — permissions + `outputStyle` only, **no `hooks` key**. Track B creates it.
  - `/Users/natemccoy/rust/hana/docs/hana/tool-graph.md` — 37 `**Files:**` blocks; 21 done, 19 todo, 20 Work Orders.
  - `/Users/natemccoy/rust/hana/docs/hana_valence/arrangements.md` — 32 `**Files:**` blocks; 23 done, 9 todo, 9 Work Orders. 19 + 9 = the 28 to backfill.
  - A `**Files:**` block on disk (arrangements.md Phase 24) — bold label, blank line, `- ` bullets, backticked **repo-relative** path (brace expansion allowed), em-dash, description:
    ```markdown
    **Files:**

    - `Cargo.toml`, `Cargo.lock` — `bevy_tween` features and lockfile refresh.
    - `crates/hana_animation/src/{lib,plugin,transport,context}.rs` — transport API and schedule.
    ```
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Integration test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth <target>` — the package `test` line above runs `--bins` only and silently skips every `crates/cargo-berth/tests/*.rs` target. A phase that adds to or relies on an integration target must run this line naming it (phase 2 established `ledger`), or that target does not run at all.
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `phase-end /clippy style-only auto-proceed`
- **Invariants:**
  - **The design is in this file.** `## Design` and the R1–R69 findings register below are
    the specification, not background. A Work Order Spec that cites a section or a finding id
    means: open `docs/berth-plan.md` in the cargo-liner checkout and read it before writing code.
    Where a finding corrects an earlier one the later finding wins, and its title says so.
  - **Track-A phases run in the `cargo-liner` repository. Track-B phases run in the `hana` repository.** Track-A paths in this plan are repo-relative and resolve against whichever checkout the phase is dispatched into — this work runs in a worktree, not necessarily in `/Users/natemccoy/rust/cargo-liner` itself, and that main-branch checkout sits at a different commit with none of this work in it. Never resolve a track-A path against an absolute repository root. Every phase states its repo in its Goal. A track-A phase that has to explain a Work Order means the boundary is wrong.
  - **Track-B phases compile nothing** and have no `verify.sh` line. They verify by exercising the artifact: run the hook shim against a synthetic JSON payload on stdin and assert the decision, `taplo fmt --check` the TOML, JSON-validate the edited settings file, and confirm every backfilled `**Reservations:**` block parses and is lexically valid. Phase 11 — the last track-A phase — installs the verified binary and records its version and absolute path; no track-B phase runs `cargo install`, `cargo build`, or any other compile.
  - **The executable is named `cargo-berth`, and `berth` is not a command.** Cargo's subcommand convention means `cargo berth <verb>` and `cargo-berth <verb>` both work, and phase 1 shipped both spellings. Every script, hook shim, skill, and acceptance gate invokes `cargo-berth <verb>` directly; use `cargo berth <verb>` only where the surrounding text is deliberately showing the Cargo spelling to a reader.
  - **Every command that cannot read the ledger fails without facts.** It exits `4`, its payload is `no_facts`, its legacy reservation fields are empty, and its message says what is wrong. A read may still proceed on the caller's judgment; a mutation and an integration establish nothing. This is inherited by every verb — no phase restates it, and every phase's gate asserts it once for the verb it adds.
  - **The mutation lock is the only thing preventing interleaved writes.** The plan once assumed a journal record smaller than `PIPE_BUF` appends atomically. `PIPE_BUF` is 512 bytes on this filesystem, the record limit is 16 KiB, and `PIPE_BUF` governs pipes rather than regular files in any case. Every write goes through the locked transaction wrapper; there is no size below which a bare append is safe.
  - Workspace lints are inherited, never restated. Denied: `clippy::{unwrap_used, expect_used, panic, unreachable, allow_attributes_without_reason, self_named_module_files, undocumented_unsafe_blocks}`, groups `all`/`cargo`/`nursery`/`pedantic` at `priority = -1`, `rust::missing_docs`, `rust::unsafe_code`. Every `#[allow]` carries a `reason = "..."`. Use `module/mod.rs` directory form when a module has submodules.
  - Every dependency is `{ workspace = true }`; versions live only in the root `[workspace.dependencies]`.
  - **The append-only journal is truth.** `journal.ndjson` is written `O_APPEND` in records capped at 16 KiB (`ledger/constants.rs`), and nothing rewrites it. One truncation is deliberate and is not an exception to append-only: replay discards a single incomplete trailing record — the signature of a crash mid-append — and treats every other malformed line as corruption. `reservations.json` is a disposable projection — rebuildable by replay, deletable at any moment. No code treats it as authoritative or as the only copy of a fact.
  - **The edit-hook path does no git subprocess work.** It replays the journal and validates the projection against what replay found — the projection alone is never the source of truth, even on the fast path — blocks solely on tier-1 foreign-branch overlap, and is silent otherwise. Reconciliation touches git and runs at SessionStart, before stateful verbs, and before checkpoint/integration — plus one retry when the cache already says block.
  - **`cargo-berth` never reads a Work Order or any hana-specific format.** No markdown parsing, no plan-doc awareness, no phase numbering. Its interface is paths and reservation ids.
  - **It publishes to crates.io**: a README for a stranger ships in v1; the crate keeps its own version and `CHANGELOG.md`; no path-only dep without a `[[publish_path_pins]]` entry.
  - Ledger loss fails **open for editing, closed for integration**. Stale/orphaned reservations are flagged, never auto-removed. `Cargo.toml`, `Cargo.lock`, and individual `.claude/config` files take **ordinary exact exclusive reservations** for the phase's duration — R34 and final D3 withdrew the announce-not-claim rule, because announcing permits exactly the concurrent edit it names. Verify-only paths stay in `**Files:**` and out of `**Reservations:**` entirely. The trunk gate ships observe-only; `CARGO_BERTH_BYPASS=1` is evaluated before any ledger read.
  - `cargo-berth` does not coordinate its own construction, and the gate installs in hana last.

## Phases

### Phase 1 — Crate scaffold and the frozen command surface  · status: done

#### As-built

`crates/cargo-berth` is a `cargo-liner` workspace member whose binary answers to both `cargo berth <verb>` and `cargo-berth <verb>` — argv normalization in `main.rs` drops the word cargo injects. Seven verbs parse and each takes `--json`: `init`, `board`, `check <paths>...`, `claim <paths>... [--before|--after|--defer|--override <blocker>] [--why <text>]`, `release <reservation-id>`, `sequence <first> <then> --why <text>`, `integrate <reservation-id> [--force --why <text>]`. Every verb returns the frozen six-field envelope `{ verb, status, exit_code, reservations, blocked_by, message }` with `status: "unimplemented"`; without `--json` only the message line prints. `BerthExit` fixes the exit table at `0` clear, `1` blocked by overlap, `2` blocked by an unsatisfied ordering edge, `3` needs user authorization, `4` ledger unreadable (fail-open for edit paths, fail-closed for `integrate`), `5` usage error, and deserializes through a checked `TryFrom<u8>` so an out-of-table code fails to parse. Seven identifier newtypes — `ReservationId`, `WorktreeId`, `CoordinationRunId`, `EdgeId`, `EventId`, `Generation`, `SchemaVersion` — serialize as bare scalars with opaque `Display`; `WorktreeId` is `pub(crate) struct WorktreeId(String)`. Dependencies are `clap`, `serde`, `serde_json`, dev-dep `tempfile`. Nothing reads or writes a ledger, no id is minted, and there is no engine.

**Files:**

- `crates/cargo-berth/Cargo.toml` — `version = "0.1.0-dev"`, `[lints] workspace = true`, `readme` key, and every dependency inherited with `workspace = true`. The root manifest needed no edit: its `members = ["crates/*"]` glob picks the crate up, and `clap`, `serde`, `serde_json`, and `tempfile` were already declared there.
- `crates/cargo-berth/README.md` — three-line placeholder.
- `crates/cargo-berth/src/main.rs` — dual-spelling argv normalization, dispatch, `fn main() -> ExitCode`.
- `crates/cargo-berth/src/cli.rs` — the frozen seven-verb clap surface.
- `crates/cargo-berth/src/exit.rs` — `BerthExit` and the documented exit table in its `//!` block.
- `crates/cargo-berth/src/ids.rs` — the seven identifier newtypes.
- `crates/cargo-berth/src/output.rs` — `OutputEnvelope`, `CommandVerb`, `OutputStatus`, and the hand-written renderer.

**Binds later work:** Exit codes 1–4 are declared but unreachable; each is owned by the phase implementing its condition and asserted end to end there. `OutputStatus` has exactly one variant — the first engine phase adds the variants it needs and replaces `status: "unimplemented"` for the verbs it implements. `OutputEnvelope::unimplemented_json` is scaffolding: the first phase returning real reservation data deletes it and returns to `serde_json`, and must then decide what exit code a serialization failure carries, because the frozen table has none. A phase needing the primary-vs-linked worktree distinction introduces it with an encoding its own consumer justifies. `tempfile` is declared and unused; the ledger tests are its first consumer. Phase 11 replaces the README placeholder. `--json` absent is covered by smoke only; the phase adding real output adds that test.

**Gotchas:** The envelope is rendered twice — by hand in `unimplemented_json` so that exit 0 provably means a complete envelope reached stdout, and by serde; `every_verb_renders_the_same_bytes_through_both_encoders` is what keeps `CommandVerb::json_name` agreeing with `rename_all`, and the typed payload is built only from owned closed types so serialization stays infallible. Any new exit code must be added to `BerthExit`'s checked `TryFrom<u8>` or JSON carrying it fails to parse. `mod ids;` carries one `expect(dead_code)` holding six identifiers with no consumer yet; it starts erroring as they gain consumers and must be narrowed or removed then, never widened. The workspace `cargo` lint group requires a `readme` key on every crate and a path outside the package root cannot be packaged, which is why the placeholder is crate-local. The executable is `cargo-berth` and `berth` is not a command.

**Ruled out:**

- R51's `WorktreeId::{Main, Linked}` enum — an untagged unit variant serializes as `null` and the two spellings display identically.
- Adding `uuid` here — nothing in this phase mints an id.
- Declaring the eight extra dependencies the cargo-tile manifest shape suggested.
- Per-verb envelopes replacing the frozen six fields — an additive typed-payload field is added instead, leaving the six untouched.
- Making the edit hook deny an *unclaimed* Edit or Write — it blocks solely on tier-1 foreign-branch overlap; R38/D5's mandatory coverage is the dispatcher refusing a Work Order with no `**Reservations:**` block.
- Renumbering phases 12–17 for R38's skill → backfill → dispatcher → hooks ordering — splitting hook construction from hook registration gives the same guarantee without rewriting every cross-reference.
- A dry-run multi-candidate engine API — offline declaration comparison runs through the shared validator, while `cargo-berth check` tests one footprint against *live* reservations.

### Phase 2 — Journal, projection, and the mutation lock  · status: done

#### As-built

Durable storage lives under `$(git rev-parse --git-common-dir)/cargo-berth/`. An
append-only NDJSON journal holds a closed operation union at one schema version,
capped at 16 KiB per record; replay repairs a partial trailing record by
truncating to the last complete one and treats a corrupt complete record as a
hard error naming its line. Replay rebuilds `reservations.json`, a projection
carrying a generation counter and published through a temp file; a reader rereads
only when the projection's generation exceeds the one it observed. A file-based
mutation lock guards every write, and a v1 transaction wrapper stamps the stored
`repo_instance_id` rather than accepting one from a caller.

`init` resolves the repository root itself, so an invocation from a nested
directory writes exactly one `.claude/config/berth.toml` at the root, with
`trunk`, `maximum_reservations`, `maximum_ordering_edges`, and `gate_mode`. It is
idempotent, leaves an edited config byte-identical, fills defaults for omitted
keys, honours a `#` inside a quoted value, and rejects an unknown key.

`ledger_unreadable` is a terminal outcome: exit `4`, an `OutputPayload::NoFacts`
payload, and agreement between process exit status, envelope `exit_code`, and
semantic status in both text and JSON mode. Scalar wire fields are validated
types — `GitObjectId` (40- or 64-character hex, so SHA-256 repositories work),
`ReservationScopePath`, `ReservationRevision`, `JournalByteOffset`, `RecordedAt`,
and an RFC 4122-variant `ReservationId` — each serializing as a plain JSON scalar.

**Files:**

- `crates/cargo-berth/src/ledger/mod.rs` — ledger open and initialize, the v1 transaction wrapper, `WorktreeIdentity`, `repo_instance_id`.
- `crates/cargo-berth/src/ledger/journal.rs` — the operation union, the writer, replay, truncation repair.
- `crates/cargo-berth/src/ledger/projection.rs` — the projection, its generation, the atomic publish, the generation-aware reader.
- `crates/cargo-berth/src/ledger/lock.rs` — the mutation lock and its RAII guard.
- `crates/cargo-berth/src/ledger/constants.rs` — file names, journal limits, and the one schema version the writer and reader share.
- `crates/cargo-berth/src/config.rs` — the `berth.toml` reader, including a quote-aware comment stripper.
- `crates/cargo-berth/src/git/` — the git subprocess helper and its constants.
- `crates/cargo-berth/src/ids.rs` — the validated identifier and scalar types.
- `crates/cargo-berth/tests/ledger.rs` — the integration target.

**Binds later work:** Every mutation runs inside the v1 transaction wrapper,
which is what stamps repository identity and advances the projection generation —
never the journal writer directly. `ledger_unreadable` / `no_facts` is inherited
by every verb. `GitObjectId` guarantees a 40- or 64-character hex oid; there is no
`CommitOid` type. `output.rs` no longer has a hand-written encoder.

**Gotchas:** `verify.sh test <package>` runs `--bins` only and silently skips
every `tests/*.rs` target — an integration target runs only when named. `PIPE_BUF`
is 512 bytes here and governs pipes rather than regular files, so the mutation
lock is the sole anti-interleaving mechanism and no record size makes a bare
append safe. Git repositories can use SHA-256 object ids, so no fixed-width
assumption about an oid holds. The schema version must have exactly one
definition: writer and reader disagreeing about it is the failure the constant
exists to prevent.

**Ruled out:** Giving `AuthorizedOverlap` the requesting reservation's
generation. A `sync doctor --lock` diagnosis surface in v1 — the lock timeout
message is the whole surface. A dedicated ledger-repair verb — recovery reuses
`init` behind an explicit confirmation flag.

### Phase 3 — The ledger's transaction surface  · status: done

#### As-built

- `Ledger::transact(worktree_id, coordination_run_id, validate)` — with
  `validate: FnOnce(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>` —
  acquires the mutation lock, replays the journal, hands the caller a
  `ReplayedLedgerState` exposing `events()`, `generation()`, and
  `journal_end_offset()`, and appends only on `TransactionValidation::Append`,
  returning `LedgerTransactionOutcome::{Appended(Box<JournalEvent>), Rejected(Rejection)}`.
  A rejected validation appends nothing by construction.
- `Ledger::open` attaches to an existing ledger without creating one;
  `Ledger::initialize` remains the only path that creates one.
- Projection reads return `ProjectionSynchronization::{Current, RebuildRequired}`
  from `projection::read_validated`, and `transact` publishes the projection even
  on the reject path when it is stale. A projection claiming a newer generation
  or more journal bytes than the journal holds is denied as
  `ProjectionError::CacheAhead` rather than retried or overwritten.
- `MutationLock::acquire(&Path, Duration)` takes the deadline as a per-call
  argument rather than a global constant; mutating verbs pass
  `MUTATING_VERB_CONTENTION_TOLERANCE` (5s). Timeout is a distinct, fact-free
  `LedgerTransactionError::LockContention` worded "another cargo-berth operation
  is still running; wait for it to finish, then retry" — naming no path, pid, or
  lock state.
- `JournalAppendError::RecordTooLarge` / `CorrectableTransactionInput::RecordTooLarge`
  separates an oversized record (too many scopes, overlong provenance or reason)
  from an unreadable ledger.
- The claim record's payload ships with per-path `ScopeKind`, an opaque
  `WorkPlanPhase(String)` in place of a `u32` phase, and 23 role-specific journal
  newtypes re-exported `pub(crate)` from `ledger/mod.rs`.
- `ReservationScopePath` (in `ids.rs`) rejects empty / `.` / `..` / `.git`
  components, absolute input, Windows drive prefixes, and backslashes — purely
  lexically, so a path that does not exist on disk is still accepted.
- `EditAuthorization::{Identified(CoordinationRunId), Unidentified}` resolves from
  `CARGO_BERTH_RUN`, else the `cargo-berth-run-id` marker file in the worktree's
  administrative directory, and fails closed to worktree-level-only recognition
  when neither is present.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs` — `Ledger::{initialize, open, transact}`, `LedgerTransactionOutcome`, `LedgerTransactionError`, `EditAuthorization`, the crate-visible newtype re-exports.
- `crates/cargo-berth/src/ledger/journal.rs` — the claim record payload and its role-specific types, `RecordTooLarge`.
- `crates/cargo-berth/src/ledger/projection.rs` — `read_validated` and `ProjectionSynchronization`.
- `crates/cargo-berth/src/ledger/lock.rs` — `MutationLock::acquire(&Path, Duration)`.
- `crates/cargo-berth/src/ledger/constants.rs` — `COORDINATION_RUN_MARKER_FILE_NAME` ("cargo-berth-run-id"), `COORDINATION_RUN_ENVIRONMENT` ("CARGO_BERTH_RUN"), `MUTATING_VERB_CONTENTION_TOLERANCE` (5s).
- `crates/cargo-berth/src/ids.rs` — tightened `ReservationScopePath`, `WorkPlanPhase`.

**Binds later work:** `ReservationScopePath::from_str` already rejects every
invalid form, so later phases validate no paths themselves. `WorkPlanPhase`
carries opaque text, not `u32`. The `cargo-berth-run-id` marker's lifecycle —
claim writes it, release removes it, reconciliation treats an orphan as stale —
belongs to later phases; nothing writes, removes, or sweeps it yet. `pub(crate)`
items are unreachable from a `tests/` integration target because `cargo-berth` is
a binary crate, so coverage of them lives in unit tests and any test driving them
from outside the crate must run the built binary.

**Gotchas:** `Path` equality is component-wise, so
`PathBuf::from_iter(p.components()) == p` is a tautology that proves no
normalization — `CanonicalWorktreeRoot::from_str` and
`WorktreeAdministrativeLocator::from_str` compare rendered strings instead. Under
`-D warnings`, an item with no in-crate caller needs
`#[cfg_attr(not(test), expect(dead_code, reason = "..."))]`, and removing its only
test-build caller breaks the test-profile lint even with the attribute in place.
The ledger directory is `.git/cargo-berth/`, not `.git/berth/`.

**Ruled out:** directing lock-contention errors at `cargo-berth sync doctor --lock`
— that subcommand is absent from the frozen surface and out of v1, so the message
says only to wait and retry. Retrying or implicitly repairing a cache-ahead
projection — denial is the rule, and recovery is an explicit, separately-owned
user-invoked command. A second global lock-timeout constant for the edit-hook
path — the deadline is a per-call parameter and that path has no caller yet.

### Phase 3b — Claims, scopes, and overlap  · status: done

#### As-built

`claim` and `check` are real verbs in both output modes. `claim` runs one locked transaction through `Ledger::transact`: replay journal events into live reservation state, compute overlap against every live foreign reservation, reject before appending on conflict (exit `1`), otherwise mint a `ReservationId`, append, and publish the `cargo-berth-run-id` coordination marker. `check` never locks, never calls git, and never mutates — `Ledger::read_for_edit_check` locates the ledger by walking the filesystem instead of shelling out to git, answers from journal truth when the projection is absent or behind, and returns no facts rather than repairing when it is ahead or corrupt. Overlap is path-component ancestry, not string prefix (`crates/hana_kana` does not match `crates/hana_kana_extra`), through the pure pair `paths_equal` / `path_is_component_ancestor`, folding case via `PathCase` read from `core.ignoreCase`; scope sets reduce to a minimal antichain. Scopes validate lexically and never through the filesystem, so a path that does not exist yet is a valid claim, and `ScopeKind::{File, Tree}` carries the declared meaning rather than inferring it from disk. Provenance is typed at the clap boundary: paired `--plan`/`--phase` become `ClaimSource::WorkPlan`, `--run` a `CoordinationRunId`, `--head` a `ProtectedPhaseStartHead`, and `--why` a `ReservationPurpose` whose `NotProvidedByCaller` variant names its own absence instead of carrying placeholder text; an unpaired `--plan` or `--phase` is a usage error (exit `5`), as is any lexical scope rejection. A blocked claim's `ReservationConflict` payload names every holder — reservation, branch, plan, phase, reason — and the message pluralizes across zero, one, and many.

**Files:**
- `crates/cargo-berth/src/scope/mod.rs` — the whole scope vocabulary (`ScopeKind`, `ReservationScope`, `ReservationScopeSet`), `PathCase`, lexical validation, overlap policy; `ledger/journal.rs` re-exports it, and there is exactly one definition.
- `crates/cargo-berth/src/scope/antichain.rs` — component ancestry and minimal-antichain reduction.
- `crates/cargo-berth/src/reservation/mod.rs` — `LiveReservationSet::replay`, deriving live state from `JournalOperation` events; `ReservationReplayError` on an inconsistent set.
- `crates/cargo-berth/src/verb/claim.rs` — the locked acquisition transaction and marker publication.
- `crates/cargo-berth/src/verb/check.rs` — the mutation-free, git-free tier-one check.
- `crates/cargo-berth/src/ledger/mod.rs` — `Ledger::read_for_edit_check`, the filesystem-walking read surface.
- `crates/cargo-berth/src/output.rs` — `ClaimPayload::{Claimed, Blocked}`, `CoordinationRunMarkerPublication::{Published, Unavailable { diagnostic }}`, conflict payload.
- `crates/cargo-berth/tests/overlap.rs` — built-binary coverage of the overlap matrix; `cargo-berth` is a binary crate, so crate-private transaction coverage stays in unit tests.

**Binds later work:** `LiveReservationSet::replay` is the only path by which live state is derived — a `JournalOperation` variant not handled there is silently dropped. `Ledger::transact` owns `mutation.lock` and applies the contention tolerance itself; no call site acquires or times it, and a rejected transaction appends nothing by construction. `check` makes no git call, takes no lock, and leaves `journal.ndjson`, `reservations.json`, and `mutation.lock` byte-identical, so anything on a per-edit path can call it; it resolves the acting run through `EditAuthorization` — `CARGO_BERTH_RUN`, then the marker file — so a caller must export the variable or rely on the marker `claim` publishes, and must tolerate a marker that never published. `ReservationConflict` carries holder branch, plan, plan phase, and reason. `--plan` and `--phase` travel as a pair.

**Gotchas:**
- A bare path means `tree:` to `claim` and `file:` to `check`; explicit `file:` / `tree:` prefixes override in both.
- `check` has no `--run` flag by design; it reads as a defect until you find `EditAuthorization`.
- The ledger transaction commits before the marker is published, so a claim can succeed while its marker does not — the outcome is a typed state on the success payload, not an error, because the append cannot be undone.
- `LedgerTransactionError::LockContention` maps to exit `4`, documented as "the ledger cannot be read"; a caller keying on exit `4` treats transient contention as an unreadable ledger.
- `JournalOperation::Widen.added_scopes` is `Vec<ReservationScopePath>` with no `ScopeKind`, so replay assigns `File` to every widened scope — a widened tree scope would stop blocking its descendants. Unreachable while no verb emits `Widen`.
- `import-types-directly` is `mechanism: mend`, so the style-guide loader excludes it from the LLM checklist and neither fmt nor clippy checks it; inline `crate::…::Type` paths pass every automated gate.

**Ruled out:** `Announce` and `ReadOnly` access modes — announcing permits exactly the concurrent edit it names. A root-manifest exception — `Cargo.toml`, `Cargo.lock`, and individual `.claude/config` files take ordinary exclusive reservations. Letting `check` call git on the blocked path to derive lifecycle state — writers materialize that state into the journal instead. Broadening exit `4` to also mean "busy, retry" — it would make a fail-open rule wrong for a transient condition. Treating marker-publication failure as a claim error — the transaction has already committed.

### Phase 4 — Reservation lifecycle and git evidence  · status: done

#### As-built

`release` ships end to end — verb, dispatch, output status, tests. Four orthogonal types carry what one stage enum would fuse: `ReservationLifecycle::{Active, Outstanding { protected_tip: ProtectedReservationTip }, Released { disposition: ReleaseDisposition }}`, `IntegrationEvidenceStatus::{NotIntegrated, Integrated { trunk_oid }, TrunkRewritten, ObjectUnknown}`, `EditBlockingStatus::{Blocking, Clear}`, and `ReleaseDisposition::{Integrated, RewrittenIntegration(RewrittenIntegrationTrunkCommit), Abandoned(AbandonmentReason), RetiredOrphan(OrphanRetirementReason)}`. No `Option` in `lifecycle.rs` or `evidence.rs`: an `Active` reservation cannot carry a protected tip and an `Outstanding` one cannot lack it, which is what makes `Active` block unconditionally.

Checkpoint records `Outstanding { protected_tip }` at the branch tip and writes retention ref `refs/cargo-berth/reservations/<id>`, so the commit survives branch deletion and `git gc --prune=now`; resnapshot replaces both. `protected_tip` — never the claim-time head snapshot, never the live branch tip — is the subject of every reachability question.

Evidence is revalidated on every stateful check, never trusted as terminal. Each revalidating verb appends `JournalOperation::EvidenceRevalidated { reservation_id, status, edit_blocking_status }`, carrying the conclusion and not only its inputs, because the pre-edit reader cannot call git to re-derive it; `verb/check.rs` holds zero git references and answers correctly with `git` off `PATH`. Writers never reuse that materialized answer: `release` recomputes from git, which is the trunk-rewrite case.

`release` retires the coordination-run marker only when it still names the released run and no other reservation of that run is active, reporting `CoordinationRunMarkerRemoval::{Removed, AlreadyAbsent, PreservedDifferentRun, PreservedMalformed}`. Retirement renames the marker to a private path before reading its content and deletes only the detached path, so a marker republished for another run in between survives.

`BerthExit::BlockedByContention` is exit `6`, mapped by `OutputEnvelope::contention`; `0`–`5` keep their exact meanings.

**Files:**
- `crates/cargo-berth/src/reservation/lifecycle.rs` — the four types, `checkpoint`/`resnapshot`/`release` transitions, `LifecycleTransitionError`, `ReleaseRevalidationSubject`.
- `crates/cargo-berth/src/reservation/evidence.rs` — `ProtectedReservationTip`, `current_head`, `current_trunk`, `integration_status`, `outstanding_integration_status`, `retain_protected_tip`.
- `crates/cargo-berth/src/reservation/mod.rs` — `RetainedReservationSet::replay` and the `Reservation` it yields: lifecycle, retained tip, trunk snapshot, evidence status, edit-blocking status.
- `crates/cargo-berth/src/git/refs.rs` — `ReservationRetentionRef`, write and delete; prefix in `git/constants.rs`.
- `crates/cargo-berth/src/verb/release.rs` — the verb, its marker plan, its committed action.
- `crates/cargo-berth/src/ledger/mod.rs` — `transact_with_committed_action`, marker publish and detached retirement, `CoordinationRunMarkerRemoval`.
- `crates/cargo-berth/src/ledger/journal.rs` — the restructured `Release` record and `EvidenceRevalidated`.
- `crates/cargo-berth/src/exit.rs`, `crates/cargo-berth/src/output.rs` — exit `6` and the contention envelope; release and evidence payloads.
- `crates/cargo-berth/tests/lifecycle.rs` — lifecycle, trunk-rewrite, git-free check, marker, and contention tests.

**Binds later work:** The replay type is `RetainedReservationSet`, not `LiveReservationSet`, and it retains released reservations rather than dropping them — which is why a released reservation whose git evidence a trunk rewrite invalidates can start blocking again. Lock contention is exit `6` (`BerthExit::BlockedByContention`), not exit `4`; it reaches every verb, `init` included, and means retry. Mutating transactions run their git side effects in a post-append committed-action stage inside `Ledger::transact_with_committed_action`, under `mutation.lock`, so the journal record commits before the side effect runs.

**Gotchas:** `git::delete_reservation_retention_ref` ships with no caller behind a deliberate `#[expect(dead_code)]` — a retention ref must outlive its release until every dependent successor is terminal. A committed action runs after the append, so a failed side effect leaves a durable record whose ref write never landed. `verify.sh test <package>` runs `--lib`/`--bins` only; an integration target needs `verify.sh test <package> <target>`, or its tests compile and never run.

**Ruled out:**
- One fused `ReservationStage` enum — independently changing facts in one enum make impossible states representable.
- A `CommitOid` type — `GitObjectId` already guarantees the hex oid.
- Storing evidence status as terminal, or storing only its inputs and recomputing the edit conclusion in replay.
- Mapping lock contention onto `BerthExit::LedgerUnreadable` — a bounded lock wait is retryable, not an unreadable ledger.
- Deleting a retention ref in `release`; retiring a marker by reading it and then deleting it by name.

### Phase 5 — Worktree liveness, reconciliation, and orphan alerts  · status: done

#### As-built

- `WorktreeLiveness::{Live, Unavailable, OrphanCandidate, Orphaned, Unknown}` and `WorktreeRelocation::{Unchanged, Relocated { current_root }}` come from `git worktree list --porcelain` parsed with `-z` and NUL-split, paths rebuilt through `OsStr::from_bytes` on Unix. Everything except `Live` retains the reservation's scopes and edges; a locked worktree reads `Unavailable`, not missing. A relocated worktree keeps its identity while its recorded root is corrected under the lock.
- `reconcile()` is one locked pass: it observes liveness, revalidates integration evidence and appends its conclusion, repairs retention refs from the recorded `protected_tip`, sweeps stale coordination-run markers, and returns a `ReconciliationReport` of alerts plus only the evidence it changed. Stateful verbs call it before resolving identity — `claim` reconciles before selecting its acting run. `check` stays git-free and lock-free on the clear path and reconciles only once, after the read-only snapshot already says block.
- `Alert::OrphanedOutstanding` carries reservation id, `BranchRefStatus`, `ObjectAvailability`, `RetentionRefStatus`, protected tip, and one `RecoverabilityVerdict` of `RecoverableFromBranch | RecoverableFromProtectedTip | CommitUnavailable`. It is durable — re-raised until a disposition is recorded — and rides the `claim`, `check`, `release`, `renew`, and `resolve` envelopes. The verdict match tests branch status first, since a branch reachable from the protected tip is evidence the branch survives, not evidence of recovery from the tip; that ordering is what makes `CommitUnavailable` earned.
- `RecoveryRequest::{Recovered, IntegratedAs(RewrittenIntegrationTrunkCommit), Abandon(AbandonmentReason), RetireOrphan(OrphanRetirementReason)}` is built at the clap boundary, so no `Option<T>` and no impossible flag combination reaches the `resolve` engine. `--retire-orphan --why` is the only route to `RetiredOrphan` and stays distinct from `Abandoned` after replay. A disposition that ends a reservation accepts `Active` as well as `Outstanding`; `IntegratedAs` still refuses `Active`, because integration evidence requires a checkpoint.
- `ReplaceReleaseDisposition` lets a released reservation whose evidence a trunk rewrite invalidated accept `resolve --integrated-as` again: a new record names both the superseded and the new disposition, replay takes the last, and the earlier release stays readable. An ordinary first `release` still refuses an already-released reservation.
- `EditAuthorization::{Environment(CoordinationRunId), Marker { coordination_run_id, worktree_id }, Unidentified}` is source-preserving. An environment identity is trusted as given; a marker identity is honored only when replay shows that run still holds an active reservation minted in that same worktree, so a stale marker cannot hand a foreign session the holder's exemption.
- `init --repair-projection` is a mutually exclusive branch, not an additive flag: it rebuilds the projection from journal truth, leaves `journal.ndjson` byte-identical, and never runs the initialize path. The `CacheAhead` denial names it. `renew` appends a `Renew` event advancing `ReservationRevision`.

**Files:**
- `crates/cargo-berth/src/worktree/{mod,liveness,identity,constants}.rs` — worktree enumeration, liveness classification, run-marker identity, path constants.
- `crates/cargo-berth/src/reconcile.rs` — the locked reconciliation pass; retention repair lives in `append_evidence_and_retention`.
- `crates/cargo-berth/src/alert.rs` — the alert domain and `for_orphaned_outstanding`.
- `crates/cargo-berth/src/recovery.rs` — the `resolve` request enum and its dispositions.
- `crates/cargo-berth/src/reservation/mod.rs` — replay, including marker-identity validation.
- `crates/cargo-berth/tests/liveness.rs` — the liveness, alert, recovery, and marker-sweep integration cases.

**Binds later work:** Every stateful verb calls `reconcile` before resolving identity, never after. `EditAuthorization`'s three sources must stay distinct wherever a caller's identity is resolved. Exit `6` (`BlockedByContention`) means lock contention in every verb including `init`, so recovery paths treat it as retry, not an unreadable ledger. `Alert::OrphanedOutstanding` and its verdicts have no rendering surface yet — `check` reports blocks, not alerts. Replay currently discards the `Renew` event's timestamp, so the freshness projection it feeds is unbuilt.

**Gotchas:**
- `berth.toml` lives at `<repo>/.claude/<config-dir>/berth.toml`, not the repo root, and must be committed before `git worktree add` or a claim from that worktree fails with `configuration I/O failed`.
- `Ledger::transact_with_committed_action` commits the journal before running its action, so validation outside the closure reads a tree that may have changed under the lock — trunk validation for `--integrated-as` runs inside it. For the same reason a failed post-append git side effect leaves a durable record of something that never happened; `reconcile` repairs it idempotently.
- `reconcile` drops the mutation lock before returning, so anything read afterward can be stale; marker validation runs inside the `ledger.transact` closure.
- `reconcile` repairs the retention ref of every retained reservation reaching `Outstanding` or `Released`. Deleting a retention ref elsewhere is undone on the next pass unless `append_evidence_and_retention` stops repairing it too.
- A blocked `check` reconciles once and then swallows a reconciliation failure, preserving the original exit `1`; a reconciliation error never converts a block into an all-clear, and that path never surfaces exit `6`.
- Marker sweeping deliberately includes locked (`Unavailable`) worktrees, whose owners still need their markers.
- `ReconciliationReport` returns only the evidence it changed and discards every liveness observation it made, and it resolves trunk separately per retained reservation.
- A `release` that removes an alert's subject must also drop that alert from the same report, or a resolved orphan is reported still outstanding.
- `RetainedReservationSet::apply_claim` drops the `ProtectedPhaseStartHead` the `Claim` record stores.

**Ruled out:** Treating a missing worktree as freeing its reservation; inferring lost commits from a deleted branch; `--repair-projection` as an additive flag beside ordinary initialization, which would let a repair rewrite configuration it was never asked to touch; newline-terminated porcelain parsing, which cannot round-trip a worktree path containing a newline.

### Phase 6 — Overlap answers  · status: done

#### As-built

`claim` collapses `--before`, `--after`, `--defer`, or `--override`, a named blocker, non-empty `--overlap-why`, and optional `--proposal` into one semantic authorization request.
A permissive request without its current `OverlapProposalToken` returns `NeedsUserAuthorization` (exit 3); the transient token is recomputed and matched after locked replay before anything is appended.
Only exactly one current conflict matching the named blocker can append the claim and durable `ConflictAuthorization`; zero, multiple, or mismatched conflicts return `Blocked` (exit 1) and append nothing.
`ConflictAuthorization::Sequence` records an embedded `EdgeId`, named blocker, direction, exact normalized scopes, and reason; `Defer` records `BothIntegrationsHeld` for the parent claim and blocker; `Override` records the unconstrained overlap.
Recorded scopes retain `ScopeKind`, are non-empty exact intersections keyed by `OverlapScopeRevision`, and unidentified requesters use `RequesterCoordinationIdentity::NotPresented`; text and JSON escalation expose the same blocker facts and consequence.

**Files:**
- `crates/cargo-berth/src/answer/mod.rs` — facade for answer behavior.
- `crates/cargo-berth/src/answer/proposal.rs` — transient proposals, requester identity, token matching, and escalation.
- `crates/cargo-berth/src/answer/conflict_authorization.rs` — durable recorded authorization variants.
- `crates/cargo-berth/src/answer/scope_binding.rs` — exact holder-revision and overlap-scope binding.
- `crates/cargo-berth/src/verb/claim.rs` — locked recomputation, exact-one validation, and append/reject transitions.
- `crates/cargo-berth/src/reservation/mod.rs` — authorization-aware edit-conflict suppression with paired holder facts.
- `crates/cargo-berth/src/cli.rs` and `crates/cargo-berth/src/output.rs` — semantic CLI conversion and matching text/JSON escalation.
- `crates/cargo-berth/tests/answers.rs` — freshness, isolation, lifecycle, journal-immutability, and durable-answer regressions.

**Binds later work:** Ordering consumes `Sequence`'s embedded edge identity, named blocker, direction, exact scopes, reason, and claim event id; integration enforces `Defer` as a symmetric hold between the parent claim and blocker; board and harness consumers never store or display proposal tokens as durable state.

**Gotchas:** An unidentified requester remains `NotPresented` across independently minted actor runs; any requester, answer, reason, candidate-scope, holder, or holder-scope change invalidates its token.

**Ruled out:** a standalone post-hoc answer verb; a `rescope` answer variant; one singular answer covering zero or several conflicts; durable token storage or display; attributing authorization to a person when only repository, worktree, and coordination-run identity are recorded.

### Phase 7 — The edge graph  · status: done

#### As-built

`sequence <first> <then> --why` converts a recorded deferral into one durable ordering edge inside the mutation lock: reconcile, replay, validate both endpoints, DFS for a cycle, append, sync, publish; it resolves its actor against the locked replay, refuses a stale coordination-run marker, and attaches reconciliation alerts to its own envelope. `OrderingEdge` carries its own `EdgeId`, both endpoints, the validated non-empty overlap scope set, an `OrderingReason` minted at the boundary, the declaring journal `EventId`, and an `EdgeDeclaration` recording whether it was born with a claim or resolved a prior deferral. `EdgeReadiness` is derived on every read and never stored — `Holding { hold }`, `Cancelled`, `Fulfilled` — where `EdgeHold` names why a successor is still held (`AwaitingPredecessorCheckpoint`, `PredecessorNotOnTrunk { evidence }`, `AwaitingSuccessorIncorporation`) and `UnintegratedPredecessorEvidence` separates `NotIntegrated`, `TrunkRewritten`, and `ObjectUnknown`, each needing a different recovery; `holds_successor()` is one structural match on `Holding`, so a later hold reason holds by construction. `RepositorySnapshot` is one observation taken under the lock carrying resolved trunk, each retained reservation's evidence, each holder's liveness, and grouped successor-head reachability; `readiness()` consumes it and issues no git call. `DeclareOrderingEdge` is gone from the journal, leaving `ResolveDefer` as the only post-hoc edge-creation operation and the phase 6 claim payload as the only initial one, with `JournalEvent::event_id` the single widened accessor. `maximum_reservations` (claim path) and `maximum_ordering_edges` (locked graph mutation) get their first enforcement as typed rejections with their own status, exit code, and recovery sentence, alongside `SequenceRejectionKind`'s eight variants and `InactiveMarkerRun`.

**Files:**
- `crates/cargo-berth/src/edge/mod.rs` — the edge record, `OrderingReason`, `EdgeDeclaration`, the derived readiness types.
- `crates/cargo-berth/src/edge/graph.rs` — adjacency, journal replay, declaration validation, `PreparedOrderingEdge`.
- `crates/cargo-berth/src/edge/snapshot.rs` — `RepositorySnapshot` and every reachability and evidence type readiness reads.
- `crates/cargo-berth/src/edge/cycle.rs` — DFS cycle detection, run under the lock.
- `crates/cargo-berth/src/verb/sequence.rs` — the locked deferral-to-edge verb.
- `crates/cargo-berth/src/output.rs` — the `Sequence` payload; `blocked_by` derives from `holds_successor()`.
- `crates/cargo-berth/src/reconcile.rs` — builds the snapshot; retention repair skips a reservation whose dependents are all terminal.
- `crates/cargo-berth/src/ledger/journal.rs` — `DeclareOrderingEdge` removed, `ResolveDefer` carries the ordering reason, `event_id` widened.
- `crates/cargo-berth/src/reservation/mod.rs` — the reservation-count limit check on the claim path.
- `crates/cargo-berth/src/git/mod.rs` — retention-ref deletion; `git_output_dynamic_with_input` no longer deadlocks while the mutation lock is held.
- `crates/cargo-berth/tests/edges.rs` — the phase's acceptance coverage.

**Binds later work:** `RepositorySnapshot` lives in `src/edge/snapshot.rs`, not `src/reconcile.rs`; it is the one value a later readiness or board read consumes, and readiness is never recomputed. `EdgeReadiness::holds_successor()` is the predicate a deny decision consumes, in place of matching variants. `MissingReadinessFact` — readiness asked about a reservation the snapshot does not carry — surfaces as an unreadable ledger, exit 4. `EdgeDeclaration::{Acquisition, DeferredResolution}` is durable in the journal and serialized in `sequence --json`. Retention-ref deletion is observable through `RetentionRefStatus`.

**Gotchas:**
- `readiness()` inspects the successor before the predecessor: a successor already at a terminal disposition short-circuits, or an ended edge keeps reporting as blocked.
- Readiness reads no liveness. `Unavailable`, `OrphanCandidate`, `Unknown`, and `Orphaned` are reversible worktree observations and cancel nothing; only a user-confirmed `Abandoned` or `RetiredOrphan` disposition cancels an edge.
- Deleting a retention ref is not enough alone — `reconcile` rewrites it from the recorded protected tip on the next pass unless retention repair also skips that reservation.
- A grouped reachability query must not let one unresolvable head discard the rest of the group's answers.
- `berth.toml` lives at `<repo>/.claude/config/berth.toml` and must be committed before `git worktree add`, or a claim from that worktree fails with `configuration I/O failed`.

**Ruled out:**
- A stored readiness field on the edge — a persisted status goes stale against a trunk rewrite, and R41 requires revalidation on every read.
- Journal variants recording that an edge became met or ended, per R60 and R68 — status is derived on every read.
- A flat readiness enum with better variant names — it leaves `holds_successor()` enumerating variants by hand, so a later state can be misclassified as permitting.
- Carrying `IntegrationEvidenceStatus` inside the hold — it makes `Integrated` representable in a place that means "not integrated".
- A graph library — adjacency and DFS are written directly, per R67.

### Phase 8 — The trunk gate  · status: done

#### As-built

- A `reference-transaction` git hook decides whether a proposed trunk update may proceed, shipping observe-only by default under the `gate_mode` config key (`GateMode::Observe | Enforce`). `cargo berth integrate` performs the trunk update through the same decision, refusing non-fast-forward moves and evaluating holds for **every** reservation entering trunk rather than only the requested one.
- `gate::decide`, `decide_hook`, and `decide_integration` take a `GateMode` and return `GateDecision` — `Clear | Observed | Blocked | PermitIssued | Forced` — each carrying the `ProjectionGeneration` validated under the mutation lock. `GateReconciliation` / `GateReconciliationAction<Decision>` in `reconcile.rs` binds the constraint projection, snapshot, generation, and proposed old/new trunk oids into one decision taken under a single lock hold.
- The read path onto the constraints is `IntegrationConstraintProjection` / `IntegrationReservationFacts` in `edge/mod.rs`, produced by the graph as `Result<IntegrationConstraintProjection, MissingReadinessFact>` and reached through `GateReconciliation::constraints()`. `MissingReadinessFact` is the fail-closed path: exit `4` with its own diagnostic, distinct from the exit `2` edge block.
- Git invokes the binary through a hidden `#[command(name = "__reference-transaction", hide = true)]` dispatch, absent from help and from the frozen surface.
- `init` installs from the `MANAGED_HOOKS` table in `gate/install.rs`, resolving the effective `core.hooksPath` rather than assuming the common git directory, and reports per hook name a `ManagedHookInstallation` carrying `ManagedHookActivationOutcome::Active { installation: Installed | Current }` or `Inactive { reason: PreservedUnmanaged | InstallationFailed { diagnostic } }`. One hook failing does not stop the others. `InitializationRequest` is three-way by construction — `Initialize | RepairProjection | ReinitializeAfterReview`.
- `CARGO_BERTH_BYPASS=1` is the unconditional release valve, evaluated before the ledger or the configuration. It records an audit fact in the journal, or — when the journal is unwritable — as a `cargo-berth-pending-bypass-<unique>.json` marker in the common git directory carrying a `BypassCause` and a `BypassOccurrenceTime` of `Known { at } | Unavailable`, the same schema from the shell script and the Rust writer. When neither destination accepts the fact the tool warns and still permits.
- Journal records were retyped before their first append — `BypassCause`, `ForcedIntegrationReason`, `SkippedIntegrationHoldSet` — so the append-only format needs no migration.
- The retry contract lives in `cli.rs`: one `TOTAL_GATE_DEADLINE` of 10 seconds across all attempts with exponential backoff from 50 ms, and an exhausted deadline as its own outcome, distinct from a clear result and from exit `4`.
- An unreadable or absent `berth.toml` permits the transaction with a loud stderr message rather than failing closed — R64's fail-closed rule governs the journal, whose loss erases a user-approved merge order.

**Files:**
- `crates/cargo-berth/src/gate/mod.rs` — transaction parsing, the constraint read, the five `GateDecision` variants.
- `crates/cargo-berth/src/gate/install.rs` — `MANAGED_HOOKS`, the activation outcome types, the generated fail-open shell script.
- `crates/cargo-berth/src/gate/permit.rs` — one-use forced permits, the environment release valve, pending bypass markers.
- `crates/cargo-berth/src/cli.rs` — the hidden dispatch, `init`'s hook installation and three-way request, the retry deadline, the best-effort stderr diagnostic; `verb/integrate.rs` holds the typed integrate request.
- `crates/cargo-berth/src/edge/mod.rs`, `edge/graph.rs`, `reconcile.rs` — the projection types, the read-only route onto them, and the single-lock gate decision.
- `crates/cargo-berth/src/git/mod.rs`, `output.rs` — fast-forward-checked ref updates and hooks-path resolution; the per-hook activation and gate/bypass payloads.
- `crates/cargo-berth/tests/gate.rs` — the gate scenarios.

**Binds later work:** A second managed hook is a second entry in `MANAGED_HOOKS`; installation is idempotent per hook name and never disturbs an unmanaged hook. `IntegrationConstraintProjection` is the only read path onto the edge graph. A `BypassCause::EnvironmentOverride` record carries no reason and names no skipped edges, so any rendering that assumes "reason plus flagged predecessor" holds only for `--force` permits. Pending bypass markers are written but nothing reads, publishes, or deletes them. The 10-second total deadline is the whole retry budget — no consumer adds a second retry layer. Exit `2` is edge-blocked, `4` unreadable or unprovable, `6` transient lock contention.

**Gotchas:**
- A diagnostic must never change the decision: `eprintln!` panics on a failed stderr write, so on a closed or full stderr the act of explaining a permitted update was what stopped it. Every diagnostic on the reference-transaction path goes through `write_reference_transaction_diagnostic`, which writes fallibly and discards the error — and observe-only's violation print is the path an ordinary repository actually exercises.
- `berth.toml` resolves per worktree through `git rev-parse --show-toplevel`, not through the common git directory, so a worktree created before the file is committed reads no configuration and reports an unreadable ledger from every command inside it — indistinguishable from an engine fault until someone checks that worktree's own checkout.
- The installed hook embeds the absolute path of the worktree it was installed from and `cd`s there *before* the bypass branch, because the bypass audit discovers its worktree from the process working directory; a commit-time check is repository-local and must not copy that shape. Its bypass-audit success condition is `status -eq 0`, not a list of exec-failure codes — the bypass branch always exits clear, so any non-zero status means the audit was never reached.
- One `git merge` fires three `prepared` reference transactions, so a single bypassed merge leaves several bypass records. Every `prepared` transaction records an environment bypass, trunk-related or not.
- A one-use permit is consumed only once git reports the transaction `committed`; an aborted `--force` leaves a live permit that still authorizes the next matching committed update.
- `init` exits `0` with `status: "initialized"` even when a hook is inactive — the ledger and configuration really were created, and the inactivity is carried in the message and the `hooks` payload.

**Ruled out:** failing closed on an unreadable configuration; filtering which `prepared` transactions record a bypass (deciding trunk-relatedness requires exactly the reading the release valve must not depend on); a second retry layer in the shims; a new exit code for a partially installed `init`.

### Phase 9 — Drift detection  · status: done

#### As-built

- `cargo berth drift` compares what a worktree changed against what it claimed and classifies the result into exactly one of four outcomes: silent when every changed path is inside this worktree's active reservation; a journalled auto-widen when an unheld path appears; a durable `incursion` record (`reservation_id`, non-empty foreign-holder set, non-empty path set, `at`) when a write landed inside a foreign edit-blocking reservation; and a report-only collision when an unheld path gained a blocker between classification and the widening lock — nothing widened, so nothing is journalled.
- `DriftComparison` selects a cheap two-command delta against the last observation (the clap default) or the full four-command comparison against the reservation's `ProtectedPhaseStartHead`, which now survives journal replay onto the reservation. The per-worktree fingerprint cache beside the ledger is disposable: missing or unreadable falls back to the full comparison rather than reporting no change.
- `DriftReservationSelection` binds the operation to one reservation — explicit `--reservation`, implicit when exactly one active reservation matches the acting identity, a usage error naming every candidate otherwise — plus `EveryActiveForPostCommit`, the hook's own selection, which takes every active reservation in the invoking worktree regardless of run.
- `JournalOperation::Widen` carries complete `ReservationScope`s, the resulting `edit_blocking_status`, and a `ConflictAuthorization`. A widen re-runs scope binding over the enlarged footprint: uncovered overlap yields `WidenScopeBinding::Blocked` and the collision report; covered overlap journals `ConflictAuthorization::Revalidated { overlaps }` naming the exact existing answers consulted, treated as edge-neutral because those ordering edges were created when the answers were given. `ExplicitWidenReason` is minted at the clap boundary. Every domain set is non-empty by construction: private field, fallible constructor, rejecting empty on both parse and deserialize.
- Reconciliation runs before classification and its alerts ride every return path, including unchanged ones.
- `init` installs a managed `post-commit` warning hook through the managed-hook registry, running the full comparison after every commit. Git discards its exit status, so it only warns and the commit always stands; `CARGO_BERTH_BYPASS=1` skips it before the ledger is read; a markerless invocation mints a transaction-only `CoordinationRunId` that reaches the journal only as an event envelope's actor.

**Files:**
- `crates/cargo-berth/src/drift/mod.rs` — classification, selection, comparison, fingerprint cache, porcelain parsing
- `crates/cargo-berth/src/verb/drift.rs` — the verb's envelope and payload
- `crates/cargo-berth/src/ledger/journal.rs` — restructured `Widen`, the `incursion` record, `CollisionPathSet`, `ExplicitWidenReason`, `WidenCause`
- `crates/cargo-berth/src/reservation/mod.rs` — `blocking_coverage_for_drift`, `bind_widened_scopes`, `apply_widen` replay, `phase_start_head`
- `crates/cargo-berth/src/answer/conflict_authorization.rs` — `Revalidated`
- `crates/cargo-berth/src/edge/graph.rs` — `Revalidated` as edge-neutral
- `crates/cargo-berth/src/gate/install.rs` — the managed `post-commit` hook
- `crates/cargo-berth/src/cli.rs`, `main.rs`, `output.rs`, `verb/mod.rs`, `ledger/mod.rs`, `answer/mod.rs` — surface and re-exports
- `crates/cargo-berth/tests/drift.rs` — all four classification rows, the hook, the counted-call budgets

**Binds later work:** Phase 9b builds on the incursion record and `EveryActiveForPostCommit`. Phase 10 renders the incursion record, the `Widen` record's `edit_blocking_status`, and `Revalidated`'s named bindings (creating no duplicate ordering edge), and must exclude the synthetic post-commit actor from its holder, liveness, and orphan sections. Phase 11 documents `drift`, both comparison modes, `--reservation`, the `post-commit` hook, and `CARGO_BERTH_BYPASS=1`. Phase 13's `PostToolUse` shim invokes `drift` with a named comparison selector, never a boolean, and reads structured payload fields. Phase 14's `/sync check` uses the full comparison. Phase 17 exercises incursions end to end.

**Gotchas:**
- Drift's foreign test and the edit gate's foreign test are deliberately different and must stay different. The edit gate asks whether a holder is a different coordination run (run only); drift asks whether a holder's claim covers the acting worktree (run **and** worktree), because the classification table keys its first row on *this worktree's* active reservation. Making them identical returns same-run/other-worktree holders to the "neither coverage nor blocker" bucket and auto-widens over them. The two predicates in `blocking_coverage_for_drift` are exact complements over the same `EditBlockingStatus::Blocking` set, so no holder falls into a third bucket.
- `EditBlockingStatus` is filtered **before** the identity predicate in `conflicts_with_holders`. That ordering is what makes `SameIdentity` fire only on the acting worktree's own currently-blocking reservations. Load-bearing.
- The fingerprint command-budget test counts the comparison's own commands, not everything the verb issues: reconciliation runs first, so its git calls are unavoidably on the path.
- `ReservationScopeSet::try_from` fails only on emptiness, so several `unwrap_or` fallbacks around it are unreachable — a crate-wide idiom predating this phase at `scope/antichain.rs:24`, not a defect to repair here.
- `crates/cargo-berth/src/drift/` is the only module directory in the crate with no submodules; every sibling splits. Durable structural debt.

**Ruled out:**
- Making drift's foreign test identical to the edit gate's — it reintroduces auto-widening over same-run holders in other worktrees.
- Journalling a collision — nothing was widened, so there is no durable fact.
- Erroring on a markerless `post-commit` — it silences the commit-time warning exactly when it matters most; `reconcile.rs:176` already mints a transaction-only run the same way.
- Recording `NoConflict` on a revalidated widen — it freezes in a claim that no answer was consulted.
- A clap surface for `WidenCause::Explicit` — no verb offers one; the variant stays for additive forward compatibility in an append-only schema.

### Phase 9b — Drift records and hook identity, finalized  · status: done

#### As-built

An incursion record carries an `IncursionIncidentId`, and `JournalOperation::ResolveIncursion` answers one; replay projects outstanding incidents, so re-observing an already-recorded, unanswered incident appends nothing and an answered incident stops being outstanding while both records stay in the journal. The answering command is `resolve <reservation-id> --incursion <incident-id>`; naming an unknown incident is a usage error.

The `post-commit` hook's reporting is split from its widening: it reports incursions and collisions across every held reservation and widens exactly one or none. `CARGO_BERTH_POST_COMMIT=1` selects that mode. The single widen target is the reservation the session mapping names, the one `--reservation` names, or the sole active candidate; with none selectable it mutates nothing and returns an outcome naming `drift --reservation <id>`.

Identity reaches a hook through a session-keyed mapping in `crates/cargo-berth/src/session/mod.rs`, stored as `session-identities.json` beside the journal and keyed by the harness session id from `CARGO_BERTH_SESSION_ID`. `session::apply_journal_event` touches the mapping on exactly three operations — `Claim` inserts, `Checkpoint` and `Release` retire — and returns `Published` for every other operation. `SessionIdentityMappingPublication::{Published, Unavailable { diagnostic }}` is reported in command payloads as `session_mapping_publication`; `Unavailable` is a degraded success — the journal is durable and the reservation is held, only the mapping is missing.

`EditAuthorization`'s resolution order is **session mapping → `CARGO_BERTH_RUN` → worktree marker file → `Unidentified`**, source-preserving throughout. The mapping is subject to the same liveness test as a marker: one naming a reservation replay shows released grants no holder exemption. An absent or corrupt mapping falls through rather than blocking an edit. `RejectionKind::InactiveSessionMapping { coordination_run_id }` is distinct from the inactive-marker rejection and carries its own diagnostic naming the coordination run, never the marker; it surfaces from `gate`, `sequence`, and `integrate`.

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — incursion incident identity and `ResolveIncursion`
- `crates/cargo-berth/src/ledger/mod.rs` — re-exports for the incident and mapping types
- `crates/cargo-berth/src/drift/mod.rs` — post-commit reporting split from single-target widening
- `crates/cargo-berth/src/reservation/mod.rs` — incursion incident projection, `EditAuthorization` resolution
- `crates/cargo-berth/src/session/mod.rs` — the session-keyed mapping and `apply_journal_event`
- `crates/cargo-berth/src/cli.rs` — `resolve --incursion`
- `crates/cargo-berth/src/verb/{release,sequence,integrate}.rs`, `crates/cargo-berth/src/recovery.rs` — publication result and inactive-mapping rejection at every mapping-touching site
- `crates/cargo-berth/tests/{drift,gate,edges,lifecycle,overlap}.rs` — acceptance

**Binds later work:** The resolution order above and the `session_mapping_publication` degraded-success contract are the surfaces rendered, documented, and consumed downstream. The mapping holds one reservation per harness session — a second claim in the same session replaces the first — it is a disposable projection replay can rebuild, and no command reads it back out (`release` takes a required positional reservation id). `DriftWideningOutcome::{Ambiguous, CoordinationRunRequired}` exist only in the command payload and are never journalled; a widen refused by a collision appends nothing either, so board-style projections from replay cannot reconstruct either.

**Gotchas:** `verify.sh test <package>` runs `--bins` only — every `crates/cargo-berth/tests/*.rs` target is silently skipped unless named, so a change to a shared invariant must name every integration target asserting it, not only new ones. Any journal operation that establishes or retires a mapping must surface its publication result; the shipped pattern is `verb/release.rs` and `ResolvePayloadSeed::into_payload`. `ResolvePayloadSeed::Recovered` safely drops the publication while `Released` carries it only because `apply_journal_event` returns `Published` for non-mapping operations — an asymmetry invisible from either call site.

**Ruled out:** Unifying the drift foreign test with the edit gate's foreign test — drift's test is run *or* worktree by design, and unifying them auto-widens over same-run/other-worktree holders. A clap surface for an explicit widen reason — no verb offers one and the frozen verb set never promised it; `WidenCause::Explicit` remains for additive forward compatibility, with empty values rejected at the parse boundary. Marker-before-environment resolution — it rewrote a passing test and broke the explicit-override escape hatch. Using the session mapping as orchestration memory — it holds one reservation per session and nothing reads it back.

### Phase 10 — The board: model, incidents, and `--json`  · status: done

#### As-built

`BoardModel::build` assembles the board from one locked replay obtained after
`reconcile()` runs and takes the repository lock — one journal generation and
byte offset back every section, so no board can show an edge as waiting in one
place and settled in another. Sections: **Ready now**, **Waiting** (each row
carries the readiness reason and its own applicable action — checkpoint wait,
each of the three not-on-trunk evidence cases, or the reader's-own-rebase case
— never a bare "Waiting"; cancelled and fulfilled edges render as settled, not
waiting), **Unresolved overlaps**, **Recorded overlap answers** (`Override`,
`Sequence`, `Defer`, each with its exact approved scopes, named blocker,
direction read from the `ConflictAuthorization` variant rather than an
optional field, reason, and consequence), a resolved-audit section for
cleanly released reservations, a holder section covering all five
`WorktreeLiveness` states (a blocking reservation with no edge and an
`Unavailable`, `OrphanCandidate`, `Orphaned`, or `Unknown` worktree is not
omitted), and alerts. Direction is absent, not empty, for answers that order
nothing. An empty graph renders "no integration order declared"; ties render
unordered. A board-visibility type (not `Option<ReleaseDisposition>`) says
which section each retained reservation belongs to, so a reblocked release
re-enters the active constraints labelled reblocked rather than reading as
still-resolved.

The projection phase 8 built for gate decisions is extended here to carry
everything a board needs, not just what a deny needs: the complete edge set
with settled and unsettled state, and the complete answer audit including
deferrals already resolved by `sequence` (which leave **Unresolved overlaps**
and appear in the audit as an ordering created from a deferral, carrying both
recorded reasons). `--json` emits the phase-1 envelope and matches the
rendered content; **this phase ships no terminal rendering** — the default
non-`--json` invocation prints a one-line pointer to `--json`, not a Debug
dump.

`AheadBehind::{Counts { ahead, behind }, Unrelated, Unavailable}` is
implemented here for the first time (`src/git/` had no prior ahead/behind
helper). `BoardGitCost` counts six per-invocation dimensions separately —
worktree listing, per-reservation evidence revalidation,
per-protected-predecessor ancestry query, per-worktree ahead/behind — each
traced independently against the git call log so a change multiplying one no
longer hides inside a single total; the true cost (larger than originally
estimated) is measured and pinned rather than batched preemptively.

Bypasses render as four distinct row kinds: three `--force` variants (skipped
only ordering edges, only unresolved deferrals, or both — each naming its
non-empty reason and flagging every skipped predecessor) and an unscoped
`BypassCause::EnvironmentOverride` row, which names the override and states
its skipped holds are unrecorded rather than showing an empty reason cell. The
several reference-transaction records one bypassed `git merge` leaves group
into one row, keyed by a write-time-captured `BypassedMergeIdentity` rather
than by coordination run. An available (uncommitted) `--force` permit renders
distinctly from a consumed one, saying retrying will consume it. A bypass
marker left when the journal was unwritable
(`cargo-berth-pending-bypass-<unique>.json` in the common git directory) is
imported on the next `reconcile()`: appended under a stable filename-derived
identity, then deleted only after the append durably lands, so an interrupted
import cannot duplicate; a marker still unwritable renders as an alert naming
the count and times instead of persisting silently.

`Alert::OrphanedOutstanding` renders reservation id, `BranchRefStatus`,
`ObjectAvailability`, `RetentionRefStatus`, protected tip, and one
`RecoverabilityVerdict`, plus the `resolve` flag that answers it, alongside
each holder's `WorktreeLiveness`; the verdict renders as given, never
inferred — "commits are lost" is earned only by `CommitUnavailable`.
Retention-ref status otherwise renders nowhere on the board except where it
changes that verdict.

Incursion incidents (`IncursionIncidentId`) render durably from both the
straying worktree and the entered reservation's worktree, one incident per
unresolved observation regardless of repeat observation, carrying the
`resolve` flag that answers it; an answered incident leaves the outstanding
section and stays in the recorded-answer audit. A drift-caused widen renders
distinguishably from a claim, shows its `edit_blocking_status`, and — when
authorized by `ConflictAuthorization::Revalidated` — names the exact existing
overlap bindings it was re-bound against, adding no second ordering edge and
no duplicate audit row. A markerless `post-commit` run's synthetic
`CoordinationRunId` produces no holder row, no liveness row, and no orphan
alert. `BoardAlert::{OrphanedOutstanding, StaleReservation, UnrecordedBypasses}`
render with resolution actions and flags; reservation freshness is derived
only from claim, widen, renew, and checkpoint events, never from unrelated
journal traffic or `HEAD` movement. Outside a git worktree, `verb::board`
exits 4 (`ledger_unreachable`) naming the cause.

**Files:**
- `crates/cargo-berth/src/board/mod.rs`, `src/board/tests.rs` — the model:
  sections, readiness, alerts, incursion projection, and an in-process
  `#[cfg(test)]` unit module reaching `pub(crate)` `BoardModel` without
  widening its visibility.
- `crates/cargo-berth/src/verb/board.rs` — the `board` verb.
- `crates/cargo-berth/src/git/mod.rs` — adds `AheadBehind` and its
  computation; no prior helper existed.
- `crates/cargo-berth/src/ledger/mod.rs`, `src/ledger/journal.rs` — the locked
  replay and generation/offset every board section reads against, and the
  bypass, permit, and deferral records the audit sections render.
- `crates/cargo-berth/src/alert.rs`, `src/reconcile.rs`, `src/edge/snapshot.rs`,
  `src/edge/mod.rs`, `src/edge/graph.rs`, `src/reservation/mod.rs`,
  `src/gate/permit.rs` — read by the board; role unchanged from prior phases.
- `crates/cargo-berth/tests/board.rs` — model and integration tests.

**Binds later work:**
- Phase 10b hardens this phase's board model: it adds a per-invocation
  recovered-bypass set (phase 13's one-time recovery report needs it and this
  phase cannot supply it), a `renew` action on `BoardAlert::StaleReservation`
  (its siblings already carry one), and rejects an unsupported journal
  `schema_version` before decoding version-specific fields — replay currently
  deserializes the whole `JournalEvent` first, so an unsupported version
  reports `CorruptInteriorRecord` instead of `UnsupportedSchemaVersion`
  (R19, R68). It also renames `ConflictAuthorization::Revalidated` and
  `RecordedAnswer::RevalidatedWiden` before phase 11 freezes their serialized
  tags — phase 11 depends on 10b, not on this phase alone, because every item
  10b carries is unfixable after the freeze without a version bump.
- Phase 10c renders this model in a terminal and must add no fact of its own;
  the board dispatch must branch on the resolved output mode in `src/cli.rs`,
  because `Command::execute` currently discards the `Board` arguments and
  calls `board::execute()` unconditionally.
- Phases 11, 13, 14, and 15 parse the `--json` payload this phase froze — its
  sections, tagged states, per-row user actions, and audit history.
- The board is assembled from one locked replay carrying one journal
  generation and byte offset; a board built from two replays can contradict
  itself across sections.
- The board renders no attribution for a widen that did not happen — replay
  holds nothing that separates a run that reported incursions while widening
  none from one with no widening to do.

**Gotchas:**
- `BypassedMergeIdentity` enforces a charset invariant at construction and on
  the deserialize path, because the installed shell hook once interpolated an
  unescaped identity into marker JSON; the hook substitutes
  `git-process-$PPID` for an invalid inherited value, silently, by design —
  the bypass must never fail on a bad identity.
- Reservation staleness needed elapsed-time arithmetic `RecordedAt` doesn't
  provide; the civil-date conversion inverse was written from scratch.

**Ruled out:**
- Grouping bypasses by coordination run — collapses two bypassed merges under
  one identified run and splits one merge's transactions under an
  unidentified run; the grouping key is a write-time identity instead.
- A board row for `DriftWideningOutcome::{Ambiguous, CoordinationRunRequired}`
  — replay cannot reconstruct either from journal state; the command's
  immediate output, phase 13's shim, and phase 11's README carry that instead.
- `Option<ReleaseDisposition>` for board section membership, and an optional
  direction field on every answer row — both replaced by types that make the
  absent case unrepresentable rather than an empty cell.
- Retention-ref status as its own board column — it changes nothing an
  ordinary reader can act on except an orphan alert's recoverability verdict.
- A precomputed cost bound ("one `git worktree list` plus at most `P`
  ancestor checks") — unachievable; the true cost is measured and pinned by a
  counted-call test instead.

### Phase 10b — Pre-publish contract hardening  · status: done

#### As-built

Four contract repairs to phases 9b and 10, landed before phase 11 freezes the
journal and board schemas.

`replay_complete_records` decodes a `JournalSchemaHeader` before any
`JournalEvent`, so a newer writer's record reports `UnsupportedSchemaVersion` —
answered by upgrading the tool — while a malformed v1 record still reports
`CorruptInteriorRecord`, which routes to confirmed reinitialization. No record on
disk changed.

`BoardModel` carries `RecoveredBypassesThisInvocation` under
`recovered_bypasses_this_invocation`: markers whose durable recovery completed
during this read, distinct from the audit history and from
`BoardAlert::UnrecordedBypasses`. Transient by construction — a later read
reports an empty set.

`reconcile::reconcile` takes `RecoveredBypassReporting::{Report, Defer}`. A
`Report` caller retires the markers inside `ReconciliationAction::commit`, which
the ledger runs while the mutation lock is held, making the claim atomic with the
read that produced it; a `Defer` caller leaves every marker and receives an empty
set. `verb/board.rs` is the only `Report` caller — `claim`, `release`, `check`,
`drift`, `sequence`, and both `recovery.rs` paths defer.
`OutputEnvelope::render_text` names each recovered marker, so the notice reaches
human `board` output and not only `--json`.

`BoardAlert::StaleReservation` carries
`StaleReservationResolutionAction::Renew { reservation_id }`, in the form
`OrphanResolutionAction` already used. `ConflictAuthorization::Revalidated` and
`RecordedAnswer::RevalidatedWiden` are both `ExistingAnswersCoverEveryOverlap`.

**Files:**

- `src/ledger/journal.rs` — schema-version gate; marker filename accessor.
- `src/reconcile.rs` — `RecoveredBypassReporting`; retirement in the locked
  committed action.
- `src/gate/permit.rs` — `RecoveredPendingBypassMarker { id, path }`;
  `prepare_pending_bypass_recovery` dedups by marker id against durable
  `PendingMarker` events.
- `src/board/mod.rs` — recovered-bypass identities, stale-alert resolution,
  renamed recorded answer.
- `src/output.rs` — human rendering of the recovery notice.
- `src/verb/{board,claim,release,check}.rs`, `src/recovery.rs`, `src/drift/mod.rs`
  — explicit reporting or deferral at every call site.
- `src/answer/conflict_authorization.rs` — renamed authorization value.
- `tests/board.rs`, `tests/drift.rs`, `src/board/tests.rs` — deferral, one-time
  reporting, human output, deduplication, unappendable markers.

**Binds later work:** `recovered_bypasses_this_invocation` and the stale alert's
`resolution` ship under those exact names. The two renamed values carry the tag
`existing_answers_cover_every_overlap` under *different* fields —
`authorization.kind` for `ConflictAuthorization`, `answer` for `RecordedAnswer`.
`board --json` exposes `payload.data.recovered_bypasses_this_invocation` as an
array of marker ids, empty on every read after the one that adopted them.

**Gotchas:** `verify.sh test <package>` runs the binary suite alone and this
package has nine integration targets, so a serialized-tag change is invisible
unless every target asserting on payload text is named. A marker file survives
indefinitely when no board read happens — by design, since it is the undelivered
notice and its audit record is already durable, which is what keeps repeated
deferrals idempotent. `claim` takes `--why` not `--purpose`, `--proposal`
requires a `<--before|--after|--defer|--override>` group, `--run` demands a
UUID-v7, and `check` requires a path.

**Ruled out:** retiring markers unconditionally inside reconciliation — every
non-board verb then drains the notice before a board can name it. Retiring them
from `BoardModel::build` after the lock released — two concurrent reads both
claim the same marker and announce it twice.

---

### Phase 10c — The board TUI  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `cargo berth board` opens a terminal view of the board model phase 10 built, with no new facts of its own.

**Spec:** A `tui_pane` front end over phase 10's headless board model. Implement `tui_pane`'s `AppContext` trait, register each pane's bindings with `KeymapBuilder::register_pane`, and use `Keymap` for dispatch and `StatusBar` for the footer. `PaneRegistry` is a separate trait — it maps a resolved pane id to its render target and is what `tui_pane::render_panes` walks; `cargo-tile` does not implement it and dispatches rendering itself, while `cargo-port` does. Follow `cargo-tile` for how a `cargo-*` binary embeds the framework, and take either rendering route deliberately rather than assuming binding registration also registers a render target.

**This phase renders; it does not decide.** Every section, readiness reason, waiting action, alert, incident, and audit row is read from the model phase 10 shipped and already proved through `--json`. Adding a fact here — a computed status, a derived label, a rule about what belongs where — means it is missing from the model, and it belongs in `board/mod.rs` where `--json` also gets it. The two output modes must not diverge.

`--json` continues to run headless with no TTY and must not acquire a terminal, so the TUI entry point is selected after the output mode is resolved, never before. **That selection belongs in `src/cli.rs`, which is the only place the resolved output mode exists.** `Cli::run` computes `CliOutputFormat` from the verb's own flag, but `Command::execute` discards the `Board` arguments and calls `board::execute()` unconditionally, so `verb/board.rs` cannot see the mode and cannot select anything. Route the resolved `CliOutputFormat` into the board dispatch and branch there, using that existing enum rather than a second output-mode type or a boolean. A human mode with no attached terminal is its own outcome and must say what to run instead — it is not a silent fall-back to JSON and not a panic.

**Files:**

- `crates/cargo-berth/src/board/tui.rs` — `AppContext` impl, panes, keymap.
- `crates/cargo-berth/src/board/mod.rs` — **exists** after phase 10; read it for the model this renders, and add the `mod tui;` declaration `board/tui.rs` requires. That declaration is the only edit this phase makes to it; every value it renders is read, never recomputed.
- `crates/cargo-berth/src/verb/board.rs` — **exists** after phase 10; runs the headless model read the terminal path also consumes.
- `crates/cargo-berth/src/cli.rs` — **exists**; `Command::Board`, `Command::execute`, and the `CliOutputFormat` resolution that must reach the board dispatch.
- `crates/cargo-berth/tests/board.rs` — **exists** after phase 10; the output-mode assertions this phase adds.
- `crates/cargo-berth/Cargo.toml` — `ratatui`, `crossterm`, and `tui_pane`, none of which this crate names yet.
- `crates/cargo-tile/src/app.rs`, `keymap.rs`, `terminal.rs`, `render.rs` — read only, **repo-relative in this workspace**; the complete worked example of a `cargo-*` binary embedding the framework: the `AppContext` impl, `register_pane` binding registration, terminal acquisition and restoration, and manual render dispatch. Read the copy in this checkout rather than any absolute path into another clone, which carries a different revision. Take `cargo-tile`'s manual render dispatch route: it does not implement `PaneRegistry`, and this phase does not either.

**Constraints from prior phases:** Phase 10 owns the board model, its sections, its incident and alert projection, and the `--json` representation; take those values rather than recomputing anything. Phase 10b adds two values to that model under these exact names: the field `recovered_bypasses_this_invocation`, typed `RecoveredBypassesThisInvocation` and holding the pending-bypass marker ids this read adopted (empty on every read after the one that adopted them), and a `resolution` field on `BoardAlert::StaleReservation` typed `StaleReservationResolutionAction::Renew { reservation_id }`. Render both like any other fact and add none of your own. Phase 10b also renamed `RecordedAnswer::RevalidatedWiden` to `RecordedAnswer::ExistingAnswersCoverEveryOverlap`, so the recorded-answer rows this renders carry that name. **The two renamed values do not share a discriminator field.** `RecordedAnswer` is `#[serde(tag = "answer")]`, so a board row reads `answer: "existing_answers_cover_every_overlap"`, while `ConflictAuthorization` is `#[serde(tag = "kind")]` and reads `kind: "existing_answers_cover_every_overlap"`. Phase 2's typed payload is the `--json` representation and this phase does not change it. Phase 1's envelope still applies. **The `tui_pane` pin already exists.** The workspace root already declares `tui_pane = { path = "crates/tui_pane" }` and `.claude/config/release.toml` already carries its versionless `[[publish_path_pins]]` entry, added for `cargo-port` and `cargo-tile`; this phase only makes `cargo-berth` a third consumer of both. No new pin entry is needed, and phase 11 must not be written as though this phase created one.

**Acceptance gate:** `verify.sh test cargo-berth` and `verify.sh test cargo-berth board` both green. `board --json` still runs headless with no TTY and its content is byte-identical to what phase 10b produced, that baseline being the hardened payload rather than phase 10's. All four output-mode outcomes are asserted: `--json` with no TTY emits the payload and acquires no terminal; human mode with a TTY enters the terminal view and restores the terminal on exit; human mode with no TTY reports an actionable outcome naming `--json` rather than failing silently or panicking; and `board` outside a git worktree still exits `4` with the typed ledger-unreadable envelope in both modes. Every section, waiting reason with its action, alert, incursion incident, and recorded-answer row that `--json` reports is reachable in the terminal view, proven against the model rather than against a rendered string. **Phase 10b's two additions get their own assertions rather than resting on "every section" and "every alert", neither of which covers them:** the recovered-bypass set is a top-level model value, not a section, and is asserted to render on the invocation that adopted a marker and to be absent on the next read; and `BoardAlert::StaleReservation`'s `resolution` is asserted to render `renew` together with the same reservation id the alert carries. No fact is computed in `tui.rs` that `--json` does not also report. `verify.sh lint cargo-berth` green.

---

### Phase 11 — README, changelog, and publish readiness  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `cargo-berth` is documented for a stranger and ready to publish, without publishing.

**Spec:** Implements `#### The README is a deliverable` in `docs/berth-plan.md` (this plan).

`crates/cargo-berth/README.md` **already exists** — phase 1 shipped a three-line placeholder because the workspace lints require a `readme` key. This phase **replaces its contents**; it does not create the file.

Written for someone who has never heard of hana, `/plan:delegate`, or Claude Code:

- The six commands in first-use order: `cargo install cargo-berth`; `cargo berth init` (creates the ledger in `.git`, installs the trunk hook and the commit-time warning hook, writes a default config); `cargo berth claim <paths>`; `cargo berth board`; `cargo berth integrate`; `cargo berth release`.
- A real collision transcript — actual output — showing which branch holds what and the four answers. It includes the full transition: a neutral claim is blocked at exit `1`, the user supplies one answer and reason to receive an exit-`3` proposal, and a separately approved invocation applies that exact token.
- **The enforcement limits must be prominent.** The trunk gate is a git hook, so merge ordering is enforced for anybody with no discipline required. Editing is different: blocking the write itself is a Claude Code `PreToolUse` hook and is *not* part of this tool; a general user gets an automatic warning at commit time instead. It is later than blocking the keystroke and it never blocks: the commit is already made when the warning appears, it names what strayed and whose reservation it reached, and the decision is the user's. `cargo berth drift` runs the same check on demand. A coordination tool that oversells its enforcement is the failure this design exists to avoid. **Describe the second gap beside the first.** A permissive overlap answer takes two deliberate invocations and records the submitting repository, worktree, and coordination run, why the answer was chosen, and the exact overlap it covers. The journal does not identify a person and cannot prove a person rather than an agent supplied the answer, because a published binary has nowhere to send an escalation its own caller cannot read. Say what it guarantees (the answer is deliberate, reasoned, scoped to the conflict it was shown, attributed to the submitting coordination identity, and visible on the board) and what it does not (a person's identity or that a human was in the loop), and name the invoking harness as where that last part is enforced. **Enforcement is conditional, and the README must say under what conditions.** As shipped, the trunk gate defaults to observe-only: it reports a violation and permits the merge, and enforcing is a configuration choice. It also fails open in three further ways by design, each of which a reader can hit without doing anything wrong. A `berth.toml` that cannot be read permits the update and explains itself. A `cargo-berth` executable that is missing or cannot run permits the update and explains itself. And an unrelated `reference-transaction` hook already owning the name is preserved rather than replaced, which means `init` succeeded and the gate is not installed — `init` reports that per hook, in its message and in its `hooks` payload, and the README says to read it. Name the route back for each: restore the configuration file, rerun `cargo berth init`, and for the preserved hook, incorporate the existing hook in a wrapper or move it aside and rerun `init`. **Fail-closed is about the journal, not about everything.** Integration fails closed on an absent, corrupt, or unknown-epoch journal, because losing the journal erases a merge order the user approved. A configuration file that cannot be read is a different thing. State both rules together and say why they differ, so a user who hits the second one does not read it as the gate having failed open by accident. - **`drift` gets its own section, because every track-B phase is written against it and nothing else documents it.** Name the default cheap comparison and what `--full` selects instead, and say plainly what each costs and which question each answers. Name `--reservation`, the implicit single-active-reservation rule, and the ambiguous-selection usage error that names every candidate. Name the `post-commit` hook's different subject rule — it checks every reservation the invoking worktree holds rather than refusing on ambiguity — and `CARGO_BERTH_BYPASS=1` skipping it. Document each of the three consequences a drift result can carry: an auto-widen that grew the reservation, an incursion that reached a foreign holder, and a collision that refused to widen. **Measure the real cost of a complete `drift` call and publish that number**, because D1's `~0.02s, no lock` describes the fingerprint comparison alone while the shipped verb reconciles first under the mutation lock; the measured bound for the whole call is what phase 13's shim is written against.

- **Document the release valve and what it does and does not record.** `CARGO_BERTH_BYPASS=1` permits the update no matter what else is broken. Taking it records an audit fact — in the journal when the journal is writable, otherwise as a marker file for a later session to report — and that fact names the override and when it was taken but carries no reason and names nothing it skipped, because it is recorded before anything else is read. Say that, and say that when neither destination is writable the tool warns and still permits.
- **What a claimed path actually covers**, because the defaults differ per verb and nothing else tells a reader: a bare path given to `claim` reserves the whole tree beneath it, a bare path given to `check` asks about that one file, and `file:` / `tree:` prefixes override both. Overlap is by path component, so `crates/foo` and `crates/foobar` are unrelated; a path that does not exist yet is a valid claim; and comparison ignores case when the repository sets `core.ignoreCase`. Every one of these is surprising exactly once, and only in the direction that grants or blocks more than the reader expected.
- **The board is a documented contract, not just a command.** `--json` is what phases 13, 14, and 15 parse and what this phase freezes, so the README documents it as a payload a stranger can write against: every section and what puts a reservation in it, every tagged state a row can carry, the user action each alert and waiting reason names, the recovered-bypass set as a notice reported once by the read that completed the recovery and empty on every read after it, and the audit history as the durable half that stays visible without being re-announced. Show one real `board --json` payload and walk its fields.
- The config file, field by field.
- **`--help` is documentation too**, and the surface an agent actually reads. Audit every verb's help text against the README: each must name its flags and what they do, and any help that only restates the verb's own name gets rewritten here.
- What it deliberately does not do: choose the merge order, track phases, span repositories.

Add the `cargo-berth` row to `README.md` under `## workspace members`, matching the existing row shape. Create `crates/cargo-berth/CHANGELOG.md` in the shape its siblings use. Confirm no path-only dependency was introduced without a `[[publish_path_pins]]` entry in `.claude/config/release.toml`. **Do not run `cargo publish`** — publishing waits until track B proves the loop.

**Stage the artifact for track B.** Track-B phases compile nothing, so the binary must exist before phase 12 runs and no track-B phase may build it. Install it here — `cargo install --path crates/cargo-berth` from the track-A checkout this phase runs in — and record the resulting version and the absolute path to the installed executable in this Work Order's as-built notes. Phases 12–17 invoke that executable by name (`cargo-berth`) and never compile.

**Files:**

- `crates/cargo-berth/README.md` — **exists** as a phase-1 placeholder; replace its contents.
- `crates/cargo-berth/CHANGELOG.md` — new.
- `README.md` — one member row.
- `.claude/config/release.toml` — read only. Its versionless `[[publish_path_pins]]` entry for `tui_pane` **already exists**, added for `cargo-port` and `cargo-tile`; phase 10c only makes `cargo-berth` a third consumer. Confirm the entry covers this crate and add nothing. `/release` resolves that pin from crates.io at release time, which is why the publish check below is run through the repository's release flow rather than through Cargo directly.
- `crates/cargo-berth/src/cli.rs` — read for the real verb and flag set the README documents; edited only where a verb's `--help` text needs rewriting.
- `crates/cargo-berth/src/config.rs` — read for the config fields the README documents field by field.
- `crates/cargo-berth/src/gate/install.rs` — read for the managed-hook registry, the preserved-unmanaged case, and the fail-open behavior of the installed hook script.
- `crates/cargo-berth/src/gate/permit.rs` — read for the environment release valve, its audit destinations, and the pending marker it leaves when neither accepts the fact.
- `crates/cargo-berth/src/output.rs` — read for the per-hook activation payload `init` emits and the exact wording of its inactive-hook messages.
- `crates/cargo-berth/src/board/mod.rs` — read only; the board sections, their tagged states, the per-row user actions, the transient recovered-bypass set, and the persistent audit history, all of which the README documents as the `--json` contract this phase freezes.
- `crates/cargo-berth/src/reconcile.rs` — read only; what a board read runs first, which is why the documented cost and the documented recovery both belong to `board` rather than to a separate command.
- `crates/cargo-berth/tests/board.rs` — read only; the shipped `--json` assertions the documented example must agree with.
- `crates/cargo-berth/src/ledger/journal.rs` — read only; the schema-version gate phase 10b put ahead of operation decoding, and the source of the two distinct exit-`4` diagnostics this README must tell apart.
- `crates/cargo-berth/src/answer/conflict_authorization.rs` — read only; `ConflictAuthorization` and the `kind` discriminator this phase freezes.
- `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/drift.rs` — read only; the integration assertions that pin the journal and answer tags, and the targets this phase's gate runs by name because the package-scoped `test` line does not reach them.
- `crates/cargo-berth/src/drift/mod.rs` — read for the two comparison modes, the reservation-selection rule and its ambiguity error, the post-commit all-local selection, the three consequences a drift result can carry, and the two attribution outcomes that reach the caller only in the command's own output — ambiguous candidates and an unidentified coordination run.
- `crates/cargo-berth/src/session/mod.rs` — read for the harness session mapping the README documents: the `session-identities.json` file, the `CARGO_BERTH_SESSION_ID` variable that keys it, one reservation per session, and the publication status a command reports when the mapping cannot be written.
- `crates/cargo-berth/src/ledger/mod.rs` — read for `EditAuthorization`'s shipped resolution order — session mapping, then `CARGO_BERTH_RUN`, then the worktree marker file, then `Unidentified` — which the README states in that order and no other.
- `crates/cargo-berth/src/recovery.rs` — read for the `resolve` recovery decisions, including the incursion disposition, and for the mapping-retirement status a resolve that releases reports.
- `crates/cargo-berth/src/verb/claim.rs`, `crates/cargo-berth/src/verb/release.rs`, `crates/cargo-berth/src/verb/sequence.rs`, `crates/cargo-berth/src/verb/integrate.rs` — read for where each verb reports its session-mapping publication status and where a stale mapping produces its own diagnostic rather than the marker one.

**Constraints from prior phases:** **This phase freezes the journal and board schemas, so phase 10b must be green before it runs.** Phase 10b lands the unsupported-schema-version rejection that R19 and R68 require of v1, the recovered-bypass set and stale-reservation action the board contract documents, and the two `ConflictAuthorization` and `RecordedAnswer` tag renames — each of which is unfixable after the freeze without a version bump. Document the shipped names and the shipped payload, not phase 10's; they are `recovered_bypasses_this_invocation` (a list of pending-bypass marker ids, typed `RecoveredBypassesThisInvocation`), the `resolution` field on `BoardAlert::StaleReservation` typed `StaleReservationResolutionAction::Renew { reservation_id }`, and `ConflictAuthorization::ExistingAnswersCoverEveryOverlap` together with `RecordedAnswer::ExistingAnswersCoverEveryOverlap`. **The two carry the same tag value under different discriminator fields, and the freeze must record both separately:** `ConflictAuthorization` is `#[serde(tag = "kind")]`, so the journal and claim payloads freeze `authorization.kind = "existing_answers_cover_every_overlap"`, while `RecordedAnswer` is `#[serde(tag = "answer")]`, so the board freezes `recorded_overlap_answers.entries[].answer = "existing_answers_cover_every_overlap"`. Documenting one field for both is wrong. **The unsupported-schema-version diagnostic is a distinct recovery path and gets documented beside the corrupt-ledger one, not folded into it.** A journal record written by a newer `cargo-berth` reports `journal schema version N is unsupported` at exit `4`, and the answer is to upgrade the tool — never the confirmed reinitialization that a genuinely corrupt record calls for. Phase 10b made the two distinguishable specifically so the README could tell a user which one they are looking at; a README that describes exit `4` as one condition undoes that. Every command and its real output comes from phases 1–10c as built; regenerate the transcript from the actual binary rather than transcribing this plan. The config fields are whatever phase 2's `src/config.rs` defines and `init` writes. The README documents `cargo-berth`'s real verb set, which by this point includes `drift` (phase 9) and the `resolve` and `renew` verbs phase 2's recovery-surface decision added — read the shipped `src/cli.rs`, not this plan's phase-1 table. There is no announce-not-claim behavior to document; root manifests take ordinary exclusive reservations (R34, final D3). Document the recovery paths: phase 8's — what a corrupt ledger looks like to a user, the confirmed reinitialization that restores service, and what is lost by running it — and phase 5's `cargo-berth init --repair-projection`, which rebuilds only the cache from journal truth when the cache is ahead of the journal, and loses nothing. Document all four recovery decisions phase 5 wired into `resolve`, reading them from the shipped `src/cli.rs`: `--recovered` rebinds the reservation to the worktree the command runs in, `--integrated-as <trunk-oid>` records a verified alternate commit already reachable from trunk, `--abandon --why <text>` is the only route to a deliberate abandonment, and `--retire-orphan --why <text>` is the only route to retiring a confirmed orphan and stays distinguishable from abandonment after replay. `--retire-orphan` is **not** in this plan's phase-1 table; phase 5 added it. Also document `renew`, which refreshes a reservation's freshness without touching scopes or edges. Document the enforcement limits and the two ledger-loss rules named in the Spec above, and read phase 8's shipped `gate/install.rs`, `gate/permit.rs`, and `output.rs` for the exact behavior and the exact message wording rather than describing them from this plan.

Phase 4 added exit `6` (`BerthExit::BlockedByContention`) — lock contention in **every** verb, `init` included — so the README's exit table documents seven codes, and `6` is documented as the engine's already-exhausted lock wait: the tool was busy, nothing was decided, run the command again. It is explicitly not an invitation to write a retry loop, because the ten-second deadline has already been spent inside the call that returned it. It also shipped **four** release outcomes — `ReleaseDisposition::{Integrated, RewrittenIntegration, Abandoned, RetiredOrphan}`, the last of which phase 5 makes reachable through `resolve --retire-orphan` — and the blocking `IntegrationEvidenceStatus::ObjectUnknown` that an unresolvable trunk or protected tip produces. Document all four; `RetiredOrphan` is user-actionable and a reader who only knows about `Abandoned` cannot tell them apart on the board. The README must state that an unresolvable trunk **blocks** rather than silently allowing, since that is the one case where a user sees a block with no holder to point at, and that `ObjectUnknown` keeps blocking until the missing object is restored and the evidence revalidated — it never ages out on its own.

Phase 4 also made every mutating verb commit its journal record **before** running its git side effects, so a command can report a failed ref write or marker retirement over an outcome that is already durable. The README says plainly what that means for a user: the release or checkpoint did happen, the message names a repair that did not, and the answer is to re-run the command or let the next reconciliation repair it — not to redo the work. The same paragraph documents exit `6` the same way: the tool was busy, nothing was decided, run it again by hand.

**The README documents the split `post-commit` behavior.** Phase 9b separated the hook's two jobs: it reports incursions and collisions across every reservation the worktree holds, mutating nothing, and it auto-widens exactly one reservation — the one the session mapping names, the one `--reservation` names, or the single active candidate. When it can name none it widens nothing and says so, naming `drift --reservation <id>`. Document both halves, and document the non-mutating outcome as the deliberate answer rather than a failure: the commit is already made and no reservation changed, so nothing is lost by attributing the paths by hand. Document the incursion the same way — an incident with an identity that stays outstanding until `resolve` records a disposition, not a warning that reappears forever, and name the exact command that answers one: `resolve <reservation-id> --incursion <incident-id>`.

**The README documents how a hook learns whose reservation it is acting under.** Phase 9b made identity durable rather than exported. Document `session-identities.json` beside the journal: what it holds, that `CARGO_BERTH_SESSION_ID` is the key a harness supplies, that it carries one reservation per session so a second claim in the same session replaces the first, and that it is a disposable projection the journal can always rebuild. State `EditAuthorization`'s resolution order exactly as shipped — the session mapping, then `CARGO_BERTH_RUN`, then the worktree marker file, then unidentified — and document `CARGO_BERTH_RUN` as the explicit override that outranks a marker. Document the two failures a user can meet: a command that succeeded while reporting that the mapping could not be published, which is a real success whose diagnostic tells the user to name the run and reservation explicitly afterwards; and a mapping that names a reservation no longer active, which produces its own diagnostic naming the coordination run rather than the marker one. Document that recovery from either is explicit — supply the run and reservation by name — and never a silent retry.

**Acceptance gate:** `verify.sh check cargo-berth` and `lint cargo-berth` green (`missing_docs` is denied, so the crate-level docs must be complete). **The schemas this phase freezes must be proven, not assumed.** `verify.sh test cargo-berth` runs the binary suite alone and skips every integration target, while the assertions that pin these tags live in `tests/ledger.rs`, `tests/board.rs`, and `tests/drift.rs` — phase 10b discovered exactly this gap when a serialized-tag rename broke a `drift` assertion no listed command ran. So this gate also runs `verify.sh test cargo-berth ledger`, `verify.sh test cargo-berth board`, and `verify.sh test cargo-berth drift`, and proves the journal diagnostic at the process level rather than in a unit test: a planted record carrying an unsupported `schema_version` reports `journal schema version N is unsupported` at exit `4`, a planted malformed v1 record reports a corrupt record at exit `4`, and the README is checked to send the first to a tool upgrade and only the second to confirmed reinitialization. The README's board section is likewise checked to document the recovered-bypass notice as reported once by the read that adopted it and the stale alert's `renew` action. Phase 10b's `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` are green before this phase's own gate is evaluated, since the schemas documented here are the ones it hardened. The publish check runs through the repository's release dry-run flow — `/release`, which resolves the versionless `tui_pane` pin from crates.io and diffs the local crate against what that version published before invoking Cargo's publish check — never a bare `cargo publish --dry-run -p cargo-berth`, which fails on the path-only dependency phase 10c adds. The README's board section is proven against reality: its documented example payload is compared field by field against a real `board --json` run in a scratch repository, including one section, one alert with its user action, one waiting reason with its action, and the recovered-bypass notice reported once and absent on the second read. Every command in the README runs as written against a scratch repo, and the collision transcript matches the real exit-`1` block, exit-`3` proposal, and token-bearing application output byte-for-byte. The README attributes approval only to the submitting repository, worktree, and coordination run, never to a person. Every verb's `--help` runs and names each of its flags. The README's account of the commit-time warning matches shipped behavior — it appears after the commit is made, it never blocks, and it names both the on-demand `drift` command and the environment override that skips it. The installed `cargo-berth` executable runs from a directory outside the workspace and reports the expected version. Also: the README documents `drift`'s default cheap comparison, `--full`, `--reservation` with the implicit single-reservation rule and the ambiguous-selection error, the `post-commit` hook's all-local subject rule, `CARGO_BERTH_BYPASS=1`, and all three drift consequences and both attribution outcomes that never reach the journal — ambiguous candidates and an unidentified coordination run — each transcript regenerated from the real binary; and a timed measurement of one complete `drift` call — reconciliation, lock acquisition, and comparison together, not the fingerprint alone — is recorded in this phase's as-built notes as the bound phase 13 is written against. Also: the README states that the gate ships observe-only and that enforcing is a configuration choice; it states the fail-closed rule for the journal and the permit-and-explain rule for an unreadable configuration together with why they differ; it names all three of the gate's other fail-open cases with the route back for each, including a preserved unmanaged hook and where `init` reports it; it documents all three `init` branches with their exact effects, naming confirmed reinitialization as the only route back from a corrupt ledger; and it documents `CARGO_BERTH_BYPASS=1` including that its audit fact carries no reason and names nothing it skipped, and that when neither audit destination is writable the tool warns and still permits. Also: both halves of the `post-commit` behavior are transcribed from the real binary — a single-reservation worktree whose commit auto-widens, and a two-reservation worktree whose commit reports incursions and names `drift --reservation <id>` without widening — and the README documents an incursion as an incident cleared by `resolve <reservation-id> --incursion <incident-id>` rather than a warning that reappears. Also: the README states `EditAuthorization`'s resolution order as the shipped session-then-`CARGO_BERTH_RUN`-then-marker order, documents `session-identities.json` and `CARGO_BERTH_SESSION_ID` including that one session maps to one reservation, and transcribes from the real binary both a command that succeeds while reporting its mapping unpublished and a stale mapping producing the coordination-run diagnostic rather than the marker one, each with the explicit recovery it names.

### Phase 12 — Config and init in hana  · status: todo

#### Work Order

**Goal:** In `hana`: `.claude/config/berth.toml` states this repo's dialect, and `cargo-berth init` has created the ledger with the trunk hook installed observe-only.

**Spec:** **Compile nothing.** Phase 11 already installed the verified binary and recorded its version and path; this phase invokes `cargo-berth` and never runs `cargo install`, `cargo build`, or anything else that builds. Nothing is published yet.

`.claude/config/berth.toml`, following the shape of `.claude/config/release.toml` and `mirror.toml` — a header comment explaining the tool and this repo's dialect, then per-repo policy only: `trunk = "main"`; R4's `V`/`E` limits; `gate_mode = "observe"`. **There is no announce-not-claim list** — R34 and final D3 withdrew it, and root manifests take ordinary exclusive reservations.

Run `cargo-berth init`, then confirm the ledger exists at `.git/cargo-berth/` with `journal.ndjson` and `reservations.json`, and that both managed hooks are installed where git will actually read them — the effective `core.hooksPath`, which `init` resolves and which is not always the common git directory. `init` does not overwrite the config written above. `init` takes the mutation lock, so it can exit `6` when something else holds it. The engine has already spent the whole retry budget before it returns `6` — phase 8's contract is one 10-second total deadline measured across all attempts, with exponential backoff from 50 ms inside it — so do not wrap the invocation in a second retry loop. Report the busy ledger as its own outcome, name the command to run again, and let the person decide.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/config/berth.toml` — new.

**Constraints from prior phases:** Phase 2 defines the ledger layout, the config type, and what `init` writes; phase 8 installs the hook and reads `gate_mode`. The field names here must match what phase 2's config reader expects — read `crates/cargo-berth/src/config.rs` rather than inventing them. Phase 11 installed the executable and recorded its path. `cargo-berth`'s configuration reader lives in `crates/cargo-berth/src/config.rs` in the **engine** checkout, not under this repository — name the engine checkout when citing it rather than writing a relative path that does not resolve from here. The keys `init` actually writes are `trunk`, `maximum_reservations`, `maximum_ordering_edges`, and `gate_mode`; the file is `.claude/config/berth.toml` at the repository root, and an unknown key is a hard parse error. **Commit the configuration before creating any linked worktree.** `.claude/config/berth.toml` is repository content resolved per worktree through `git rev-parse --show-toplevel`, not a single file the common git directory shares. A worktree created before that file is committed reads no configuration and every command run inside it reports an unreadable ledger, which is indistinguishable from a real engine fault until someone checks that worktree's own checkout. It cost real time during phase 8's smoke run. **Install from the durable policy checkout, not from a throwaway one.** The installed `reference-transaction` hook embeds the absolute path of the worktree it was installed from and changes to it before deciding, so that path must be a checkout that will still exist later. Run `init` from the repository's durable policy checkout. **The engine already owns the only retry budget, so a shim must not open a second one.** Phase 8 set the contract and phase 4's lock implements it: one 10-second total deadline measured across all attempts, with exponential backoff from 50 ms inside it. Exit `6` is returned only once that whole deadline is spent, so a caller that sees `6` has no budget left to retry with — a second-layer loop just multiplies a ten-second wait by however many times it runs and turns a busy ledger into a hang. Treat `6` as the named exhausted outcome it is: report that the tool was busy and nothing was decided, name the command to run again, and let the person decide. Do not write a retry loop around a verb, and do not assert one in an acceptance gate.

**Acceptance gate:** No `verify.sh`, and no compile. `taplo fmt --check .claude/config/berth.toml` passes; `cargo-berth board --json` runs and reports an empty ledger with "no integration order declared". `init` manages two hooks by this phase and resolves the effective `core.hooksPath` rather than always writing into the common git directory, so assert that `init` reports both `reference-transaction` and `post-commit` as active in its `hooks` payload and that each managed marker is present in the effective hook directory git actually reads. A clean in-order `git merge` into `main` succeeds and is **silent** — the gate ships observe-only and has nothing to say when nothing is violated, so expecting output from a clear gate is expecting the wrong thing; observe-only violation logging is phase 17's to prove. Also assert that `.claude/config/berth.toml` is committed before any linked worktree is created.

### Phase 13 — Hook shims and settings wiring  · status: todo

#### Work Order

**Goal:** In `hana`: the three hook shims exist and are proven against every exit code, ready to be switched on once the plans actually declare reservations.

**Spec:** Implements D1 — read `### D1 — RESOLVED` in `docs/berth-plan.md` (this plan).

**Write and test the shims here; do not switch them on.** R38's adoption order enables mandatory coverage and refusal **together, after backfill** — a `PreToolUse` hook active before the 28 Work Orders declare their reservations blocks nothing useful and surprises every agent that edits a path no one has claimed. So this phase creates all three scripts and proves them against synthetic payloads, and **leaves `.claude/settings.local.json` untouched**. Phase 17 adds the `hooks` key as its first step, after phase 14's skill, phase 15's dispatcher lifecycle, and phase 16's backfill are all in place.

Three shim scripts shell out to the installed `cargo-berth` binary and translate its exit codes into Claude Code hook protocol or session-start output.

`PreToolUse` on `Edit`/`Write`/`NotebookEdit`: read the tool payload from stdin, extract the target path, call `cargo-berth check <path> --json`. Exit `0` → say nothing. Exit `1` (foreign overlap) → exit `2` from the shim to block, with a message naming the holding branch, plan, phase, and reason. Exit `3` → `permissionDecision: "ask"`. Exit `4` (ledger unreadable) → **allow**; fail-open for editing is deliberate. It must make no git call and must be silent on the overwhelming majority of edits.

`PostToolUse` on `Bash`: set `CARGO_BERTH_SESSION_ID` from the hook payload's `session_id` and `CARGO_BERTH_POST_COMMIT=1` in the invocation's environment, then call `cargo-berth drift --json`. **Both are required to reach the behavior this shim is written for.** Without the session id the engine cannot resolve which reservation the harness holds; without the post-commit variable the engine selects ordinary single-reservation drift, which reports one reservation instead of every one the worktree holds and cannot produce the multi-reservation result described below. A clear result says nothing. An **auto-widen** is a notification — D1 requires reporting it so the agent knows its footprint grew. An **incursion** or a **collision** is stop feedback: surface it and tell the agent to stop. The shim inspects every `results[].effects[]` rather than the envelope's single precedence status, because one drift run carries several consequences across several reservations and precedence hides the rest. Emit through documented structured hook output; a successful command's plain stdout is debug-only and reaches no one.

**Attribution that resolved nothing is a structured outcome, not a usage error.** Read `payload.data.widening`. When it reports ambiguous candidates, or reports that a coordination run is required because nothing identified the caller, the command exits `1` carrying a `drift_attribution_required` outcome: the reporting half succeeded and every incursion and collision in the result is real, while the widening half named no reservation. Surface those results and name `drift --reservation <id>` as the way to attribute the paths by hand. Do not classify either as an ambiguous-selection usage error, and do not treat exit `1` here as a failed run — the parser distinguishes them by the `widening` field, never by the exit code alone.

**The cheap path is not lock-free and its true cost is whatever phase 11 measured.** Phase 9 ships `drift` reconcile-first, so every invocation takes the mutation lock and pays reconciliation's git calls before the fingerprint comparison runs; D1's "no lock, budget ~0.02s" describes the fingerprint alone and was never the whole path. Phase 11's measured bound for the complete reconcile-plus-drift call is the number this shim is written against and the number its acceptance asserts. This phase compiles nothing and therefore cannot repair a bound it dislikes — if the measurement is unacceptable, that is phase 11's finding to raise, not this phase's to work around.

Bash is not constrained, only observed — that is the user's D1 decision, not an oversight. Do not add a `PreToolUse` Bash matcher.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/hooks/berth_pre_edit.sh` — new.
- `/Users/natemccoy/rust/hana/.claude/hooks/berth_post_bash.sh` — new.
- `/Users/natemccoy/rust/hana/.claude/hooks/berth_session_start.sh` — new; the `SessionStart` reconciliation and alert surface, covering orphan alerts, deferred bypasses, and outstanding incursion incidents.

These shims compile nothing, but they parse concrete exit codes and payload tags, so the engine checkout phase 11 recorded is a read-only input to this phase and its exact files are named rather than left to be searched for:

- `crates/cargo-berth/src/cli.rs` — read only, in the engine checkout; the real verb and flag set, and which exit codes each verb can actually return.
- `crates/cargo-berth/src/output.rs` — read only; the envelope fields and the typed payload structures these shims parse instead of scraping prose.
- `crates/cargo-berth/src/board/mod.rs` — read only; the alert, incident, recovered-bypass, and audit tags the `SessionStart` shim reads out of `board --json`.
- `crates/cargo-berth/src/reconcile.rs` — read only; what a board read runs first, which is what makes `SessionStart` the reconciliation surface.
- `crates/cargo-berth/src/drift/mod.rs` — read only; the drift classifications, the widening outcomes, and the ambiguous-selection usage error the `PostToolUse` shim surfaces.

`.claude/settings.local.json` is deliberately **not** in this list; phase 17 edits it.

**Constraints from prior phases:** Phase 1 froze the exit codes and the six envelope fields; phase 2 added the typed payload — parse those fields, never scrape prose. Phase 9 defines the drift classifications the post-hook reacts to and adds `drift` to the verb set. Phase 5's reconcile is *not* called by the pre-hook except the single retry when the projection already says block. Phase 11 installed the executable; invoke `cargo-berth`, and compile nothing. Phase 3 owns `EditAuthorization` and its resolution, which phase 9b reordered — the session-keyed mapping first, then `CARGO_BERTH_RUN`, then the worktree marker file, failing closed to `Unidentified`. Read it; never re-derive it, and never pass a run id to `check`. Phase 3b's `claim --run` is the one provenance boundary where a run id is an argument; phase 9b's session mapping records that same value against the harness session — the same UUID-v7, not a second identifier. `--plan` and `--phase` travel together or the command line is a usage error. **A successful `claim` can still fail to publish the run marker**: phase 3b reports that as `marker_publication: unavailable` on an otherwise successful claim, and in that state phase 9b's session mapping is what lets a later `check` recognize its own run — so the shim relies on the mapping rather than on the marker existing. The concrete payloads to parse are phase 3b's `CheckPayload` and `ClaimPayload`; a blocked answer carries **every** holder, not just the first, so a message naming one holder is wrong whenever two hold the paths. Phase 4 adds exit `6`, transient lock contention. The engine has already spent phase 8's whole deadline before returning it, so the shim does not retry the invocation: it invokes once, allows the edit, and records that it allowed on an exhausted lock deadline rather than on a clear answer, keeping that outcome distinguishable from an exit `4` allow. **Phase 5 changed what a marker-derived identity buys.** `EditAuthorization` is now source-preserving — `Environment(CoordinationRunId)`, `Marker { coordination_run_id, worktree_id }`, or `Unidentified` — and a `Marker` identity is honored only when replay shows that same run still holds an **active** reservation minted in that same worktree. An environment-supplied run is trusted as given. A stale marker left by a crashed run therefore no longer grants the holder's own exemption, which is exactly why phase 9b's session-keyed mapping is the primary path and `CARGO_BERTH_RUN` and the marker file are the fallbacks behind it. **One recovery this shim reports is a one-time drain, not a standing condition.** A bypass taken while the journal was unwritable leaves a marker file that phase 10 imports into the journal and then deletes. So the `SessionStart` report is of a recovery that has just completed, and it must not be written as a condition that persists: once the markers are drained there is nothing to report, and re-reporting a drained bypass every session is the failure this note exists to prevent. **Phase 10b is what makes that readable.** Phase 10 renders an imported marker as an ordinary environment-override bypass row, indistinguishable from one journalled normally, so the board alone could not tell this shim that a recovery had just happened; phase 10b adds a per-invocation recovered-bypass set that names exactly what the current read adopted and is empty on every read after it. It reaches the shim as `payload.data.recovered_bypasses_this_invocation` in `board --json`, a JSON array of pending-bypass marker ids. Report from that set, and read the persistent bypass audit history as history — visible on the board, never re-announced at session start. `BoardAlert::UnrecordedBypasses` stays a separate report: it names the markers that still could not be journalled, which is a condition that persists until the journal is writable again. **The engine already owns the only retry budget, so a shim must not open a second one.** Phase 8 set the contract and phase 4's lock implements it: one 10-second total deadline measured across all attempts, with exponential backoff from 50 ms inside it. Exit `6` is returned only once that whole deadline is spent, so a caller that sees `6` has no budget left to retry with — a second-layer loop just multiplies a ten-second wait by however many times it runs and turns a busy ledger into a hang. Treat `6` as the named exhausted outcome it is: report that the tool was busy and nothing was decided, name the command to run again, and let the person decide. Do not write a retry loop around a verb, and do not assert one in an acceptance gate.

Exit `6` is not exit `4`, and the two must not share a branch. Phase 4 split lock contention out of `LedgerUnreadable`, so the fail-open rule keyed on `4` fires only on `4`: a busy ledger is the engine's own exhausted wait, not a lost ledger, and the shim must say which of the two it allowed on rather than collapsing them into one branch. The keystroke is still never blocked on contention — exhausting the retry allows — but it allows after actually trying, and says so, rather than mistaking a lock for a lost ledger. **The retry budget is the engine's, and it is already spent when the shim sees `6`** — phase 8's constraint explains why: each mutating call waits internally for the lock across one 10-second total deadline, backing off exponentially from 50 ms, before returning `6` at all. A shim-side loop multiplies that wait and turns a busy ledger into a hang on the one path that must stay interactive. Invoke once and report the exhausted deadline as its own outcome. **The blocked-`check` retry does not exercise what this phase's exit-`6` case assumes.** Phase 5 made a blocked `check` reconcile once and then swallow a reconciliation failure, preserving the original exit `1`, so a `check` that finds a block never surfaces `6` from that path at all; the exit-`6` case belongs to the mutating calls and the shim must be tested against real lock contention — another process holding `mutation.lock` — rather than against a synthesized status. **Nothing invokes reconciliation at a session boundary, and two shipped promises depend on it.** Phase 5 shipped `reconcile()` as a callable routine, not a process-level surface; phase 8's bypass taken while the journal was unwritable is specified to be "reported at next SessionStart"; and phase 5's orphaned-outstanding alerts are durable and meant to be re-shown until the user records a disposition. No hook registers either. Build a third shim here — a `SessionStart` script that invokes the headless board once, surfaces any orphaned-outstanding alerts with the `resolve` flag that answers each, and reports any deferred bypass marker — and leave it unregistered exactly as the other two are. **`drift` now takes an explicit reservation.** Phase 9 binds the operation to one `ReservationId` and resolves implicitly only when exactly one active reservation matches the acting identity, so the `PostToolUse` shim passes the harness `session_id` and lets phase 9b's mapping name the reservation; when the mapping resolves nothing it may use the engine's single-match resolution, but treats an ambiguous-selection usage error as a real result to surface, never as a silent no-op.

**Identity comes from phase 9b's session mapping, not from an exported environment variable.** An earlier draft of these shims assumed `/plan:delegate` could export `CARGO_BERTH_RUN` and `CARGO_BERTH_RESERVATION_ID` for a phase's lifetime and that a later hook would see them. It cannot: a Bash tool call's environment does not persist to the next call, and hooks inherit the Claude Code process environment rather than that of any command it ran. Phase 9b replaced that transport with a mapping under the ledger keyed by the harness `session_id`, written at `claim` and retired at release or checkpoint, and put it in front of `CARGO_BERTH_RUN` and the marker file in `EditAuthorization`'s resolution order. The shims therefore pass the `session_id` Claude Code already gives them and let the engine resolve; they read neither environment variable directly, and they treat an absent mapping as a fall-through to `CARGO_BERTH_RUN` and then to the marker file, exactly as the engine specifies, never as a failure. The `PostToolUse` shim still surfaces phase 9's ambiguous-selection usage error as a real result when nothing resolves, never as a silent no-op.

**`SessionStart` must report outstanding incursions, not only orphans and bypasses.** Phase 9b made an incursion a durable incident with an identity that stays outstanding until a disposition answers it, and phase 17 expects the *entered* reservation's side to learn about one at session start — that side is the only one that can judge whether the write actually conflicts, and nothing else tells it. Read the outstanding incidents from the board alongside the orphan alerts and the deferred-bypass report, and print each with the exact command that answers it: `resolve <reservation-id> --incursion <incident-id>`. An incident that has been answered must stop appearing; like the bypass drain above, this is a condition that clears, never a standing notice.

**Acceptance gate:** No `verify.sh`, and no compile. Each shim runs against synthetic stdin payloads covering every exit code the command it invokes can actually return — derived from the shipped `src/cli.rs` and that verb's engine, with any frozen code the command cannot return named as unreachable rather than synthesized — and the decision is asserted for each — including exit `6`, which the shim must treat as the engine's already-exhausted deadline and allow on without retrying the invocation itself, and must be distinguishable in the shim's own output from an exit `4` allow — including a malformed envelope, an unknown status, and a payload whose `exit_code` field disagrees with the process exit status — all three are refused rather than trusted, **except** that exit `4` still permits editing, because failing open on ledger loss is the deliberate design and must survive the stricter parsing. A real claim in a scratch worktree makes a real `Edit` block when the shim is invoked directly; corrupting `journal.ndjson` still permits editing. Also: a check run by the claiming run itself is clear while the same check from an unidentified session blocks; a block held by two reservations names both in the message; and a claim whose marker publication failed is still resolvable through phase 9b's session mapping when the shim passes the harness `session_id`, and unrecognized when neither the mapping, `CARGO_BERTH_RUN`, nor the marker file identifies it. `.claude/settings.local.json` is byte-identical to how this phase found it. Also: the exit-`6` case is driven by a second process actually holding `mutation.lock` rather than a synthesized status, and the shim's total elapsed wait stays inside the 10-second deadline regardless of how many attempts it made; the `SessionStart` shim run against a ledger carrying an orphaned-outstanding alert prints it with the `resolve` flag that answers it, run against a clean ledger prints nothing, run against a pending bypass marker with a writable journal reports the recovery once and prints nothing for it on the immediately following run, while the same bypass remains visible in the board's audit history without being re-announced, and run against a marker the journal still cannot accept reports the unrecorded-bypass condition on every run until it can, and run against a ledger carrying an outstanding incursion incident prints it with its `resolve <reservation-id> --incursion <incident-id>` command from the entered reservation's worktree, then prints nothing for that incident once a disposition answers it; and a `PostToolUse` run in a worktree holding two active reservations for one run reports the incursions and collisions across both and surfaces the `drift_attribution_required` outcome read from `payload.data.widening`, naming `drift --reservation <id>`, rather than reporting no drift or classifying it as a usage error. Also: the `PostToolUse` shim is asserted to set both `CARGO_BERTH_SESSION_ID` from the payload's `session_id` and `CARGO_BERTH_POST_COMMIT=1` before invoking `drift`, and a run with either one absent is shown to select the single-reservation path instead — the failure this assertion exists to catch. Also: a shim invoked with only the harness `session_id`, no `CARGO_BERTH_RUN` in its environment and no marker file present, resolves its run and reservation through the mapping and blocks a foreign edit accordingly; the same invocation after the reservation is released is unidentified rather than resolving a stale holder; and no shim reads `CARGO_BERTH_RUN` or `CARGO_BERTH_RESERVATION_ID` directly.

### Phase 14 — The /sync skill  · status: todo

#### Work Order

**Goal:** In `hana`: `/sync` gives the board, the checks, and the four answers, and is the only thing that reads a Work Order.

**Spec:** A skill wrapping the binary. Verbs `board`, `claim`, `release`, `sequence`, `integrate` map to the phase-1 surface. **`/sync check` is the exception and does not map to the `check` verb:** it runs `cargo-berth drift` with the full phase-start comparison selected explicitly, because it answers "did anything stray outside what was claimed". The `check` verb answers a different question — whether a proposed path or edit would collide — and stays reachable for that.

**The Work-Order-to-paths resolution lives here, not in the tool** — this is the boundary that keeps `cargo-berth` publishable. The skill reads a `**Reservations:**` block out of a plan doc and passes plain paths to `cargo-berth claim`. Grammar (R35 in `docs/berth-plan.md` (this plan)): `- file: \`Cargo.toml\`` and `- tree: \`crates/hana/src/transport\``. Paths are **repo-relative**, matching the `**Files:**` blocks already on disk.

**One shared validator, not a parser buried in this skill.** R35 requires the `**Reservations:**` block to be generated and validated by a **single** parser used by `/plan:to_phased_plan`, `/plan:phase_review`, pending-decision resolution, and `/plan:delegate` — four writers and readers that will silently diverge if each grows its own. Extract the grammar, the validation, and the pairwise comparison into one script this skill and those three commands all call.

Validation is **lexical**, never filesystem existence: repo-relative, `/` separators, no empty / `.` / `..` component, no `.git`, not absolute, lexically inside the repo, and reducing to a minimal antichain. **A path that does not exist yet is valid** — a Work Order routinely reserves the files it is about to create, and phase 3's engine takes the same position (R48).

The validator also answers *"do these two path sets collide?"* offline, from two parsed blocks alone, with no ledger and no engine call. **Offline means it reimplements the engine's rules, so it must reproduce all four or it will disagree with the tool it fronts:** overlap is path-component ancestry rather than string prefix, so `crates/hana_kana` does not collide with `crates/hana_kana_extra`; a `file:` scope and a `tree:` scope over the same spelling behave differently for descendants; a path that does not exist yet participates normally; and comparison folds case when the repository sets `core.ignoreCase`, which this one does. Phase 16's backfill needs exactly that, and `cargo-berth check` cannot provide it: `check` compares one candidate against **live** reservations, so run against an empty ledger it reports no collision no matter how badly two Work Orders overlap.

`/sync claim` owns one shared authorization state machine used at both an interactive claim and phase 15's dispatch boundary:

- **Blocked** — the neutral claim returned exit `1`. Render every current holder and shared scope, then ask the user for one direction and a non-empty reason. Phase 6's CLI answers exactly one named blocker; if several conflicts remain, require the user to narrow the requested scopes rather than pretending one answer covers all of them.
- **Proposal awaiting approval** — an answered invocation returned exit `3`. Render both plans and phases, exact shared scopes, direction, reason, and consequence, retain its proposal token as transient state, and ask for explicit approval. Do not apply the token in the turn that selected the answer or composed the reason.
- **Claimed** — the separately approved token-bearing invocation returned exit `0`. Return its `ReservationId` to the caller. The session mapping is already written by then and nothing downstream records it: phase 9b's `claim` writes the mapping itself when its own process environment carries `CARGO_BERTH_SESSION_ID`, so this skill sets that variable on every engine invocation, derived from `CLAUDE_CODE_SESSION_ID`, and a claim made without it silently produces no mapping at all. Read `session_mapping_publication` from the claim payload and treat `unavailable` as a **degraded success**, not a failure — the reservation is held and the journal is durable, only the mapping is missing, so report the diagnostic and continue with the returned `ReservationId` named explicitly from there on. Phase 9b also added a distinct `inactive_session_mapping` rejection that `sequence` and `integrate` return when the mapping names a reservation that is no longer active. It is a usage error with its own recovery — name the coordination run and reservation explicitly — and it must not be rendered as the coordination-run marker being stale, which is a different rejection with a different fix.

A token-bearing invocation that returns exit `3` is stale: discard the old token, render the refreshed proposal facts, remain in **Proposal awaiting approval**, and require another explicit approval. Exit `1` means the conflict set no longer has the one named blocker, so return to **Blocked** with the current facts. Exit `5` for a malformed proposal is not staleness: discard it, report the binary's invalid-input diagnostic, and restart from an answered invocation. `/sync` is the only workflow that may enter the token-application transition; `/plan:delegate` invokes this shared `/sync claim --from-work-order` flow rather than implementing its own proposal logic. **An agent never answers its own block** (R54).

Exit `6` is contention, not an answer, and it arrives with the retry budget already spent: the engine waits out phase 8's contract internally — one 10-second total deadline across all attempts, with exponential backoff from 50 ms inside it — and returns `6` only once that deadline is exhausted. The skill therefore does not retry the invocation. It reports the exhausted outcome as "the ledger is busy, try again" and names the command to rerun — never a clear, and never an unreadable ledger.

No mandatory emit ritual: state is pulled when wanted, never pushed on a schedule.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/commands/sync.md` — new.
- `~/.claude/scripts/berth/reservations.py` — new; the shared `**Reservations:**` parser and validator, and the offline pairwise overlap comparison, invoked by all three commands below so none of them grows a second parser (R35).
- `~/.claude/scripts/berth/claim_state.py` — new; the claim-state coordinator that turns a validated declaration into the `cargo-berth claim` invocation and reports what came back.

Both are shared artifacts with named callers, so their input and output contracts are tagged rather than nullable. A phase either declares reservations or does not, and a plan either requires them or merely advises them; encode those as `ReservationDeclaration::{Declared, Missing}` and `ReservationCoverageMode::{Advisory, Required}` — or the exact tagged equivalents in whichever language these are written in — never an absent block, an empty list standing in for absence, or a boolean. A missing declaration under `Advisory` and one under `Required` are different answers a caller acts on differently, and a caller must not have to infer which it got.

**Edit anything under `~/.claude/commands` with the Write/Edit tool, never a shell write** — it is a protected path and shell writes fail with `Operation not permitted`.
- `~/.claude/commands/plan/to_phased_plan.md`, `~/.claude/commands/plan/phase_review.md`, `~/.claude/commands/plan/delegate.md` — each calls the shared validator instead of growing its own parser (R35). `delegate.md` is also phase 15's file; keep the two phases' edits disjoint.

**Constraints from prior phases:** Phase 6 defines the four answers and that they arrive as `claim` flags, not as separate verbs — there is no standalone `override` verb. Phase 1's six envelope fields plus phase 2's typed payload are the parse target. Phase 11 installed the executable; invoke `cargo-berth`, and compile nothing. **Phase 6 shipped a blocked/proposal/apply protocol, and this skill is the only thing that performs its approval transition.** A neutral conflicting claim returns exit `1`; after the user supplies an answer and `--overlap-why`, the next invocation returns exit `3` carrying an `OverlapProposal`; only a separately approved invocation passes it back as `--proposal <token>`. The engine recomputes under the lock and distinguishes a refreshed proposal from malformed input. This skill is also where R54 is actually enforced, because the tool cannot enforce it: render the escalation payload to the user, take the direction and the reason **from the user's own answer**, and never select a permissive answer, compose its reason, or spend a token on the user's behalf. Read the shipped `src/cli.rs` for the flags. Phase 3 owns `EditAuthorization` and its resolution, which phase 9b reordered — the session-keyed mapping first, then `CARGO_BERTH_RUN`, then the worktree marker file, failing closed to `Unidentified`. Read it; never re-derive it, and never pass a run id to `check`. Phase 3b's `claim --run` is the one provenance boundary where a run id is an argument; phase 9b's session mapping records that same value against the harness session — the same UUID-v7, not a second identifier — and `--plan` and `--phase` travel together or the command line is a usage error. A successful claim returns the reservation identity, and the `claim` process itself records it against the harness session when this skill supplies `CARGO_BERTH_SESSION_ID`. Phase 13's `PostToolUse` shim passes only that session id and lets the engine resolve the reservation; it never supplies `--reservation` and never reads the mapping file. The escalation material comes from phase 3b's typed `ClaimPayload`, which names **every** holder of the paths rather than only the first, and reports each holder's provenance through `ClaimSource` — including its variant for a claim made with no plan or phase, which the escalation must render rather than assume away. **`/sync check` runs drift's full comparison, not its cheap delta.** Phase 9 splits the two: the cheap delta is the post-`Bash` hook contract and answers only whether anything changed since the last observation, while the full four-command comparison against the phase-start baseline is what a user asking "what has changed against what I claimed" actually wants. Select the full path explicitly rather than taking the verb's default. The retry budget is phase 8's single 10-second total deadline measured across all attempts, not five attempts each paying the engine's internal lock wait. **Phase 5 changed what a marker-derived identity buys.** `EditAuthorization` is now source-preserving — `Environment(CoordinationRunId)`, `Marker { coordination_run_id, worktree_id }`, or `Unidentified` — and a `Marker` identity is honored only when replay shows that same run still holds an **active** reservation minted in that same worktree. An environment-supplied run is trusted as given. A stale marker left by a crashed run therefore no longer grants the holder's own exemption, which is exactly why phase 9b's session-keyed mapping is the primary path and `CARGO_BERTH_RUN` and the marker file are the fallbacks behind it. **`/sync check` means drift's full phase-start comparison, not the `check` verb.** `/sync check` answers "did anything stray outside what was claimed", which is `cargo-berth drift` with the full comparison selected explicitly rather than the verb's cheap default. The `check` verb answers a different question — whether a proposed path or edit would collide — and stays reachable for that. The Spec states both; keep them stated in the acceptance gate too. **The engine already owns the only retry budget, so a shim must not open a second one.** Phase 8 set the contract and phase 4's lock implements it: one 10-second total deadline measured across all attempts, with exponential backoff from 50 ms inside it. Exit `6` is returned only once that whole deadline is spent, so a caller that sees `6` has no budget left to retry with — a second-layer loop just multiplies a ten-second wait by however many times it runs and turns a busy ledger into a hang. Treat `6` as the named exhausted outcome it is: report that the tool was busy and nothing was decided, name the command to run again, and let the person decide. Do not write a retry loop around a verb, and do not assert one in an acceptance gate.

**Acceptance gate:** No `verify.sh`, and no compile. `/sync board` renders; `/sync claim --from-work-order docs/hana/tool-graph.md <phase>` resolves a real Work Order to the right paths and claims them. A forced collision follows the complete shared state machine: the neutral invocation becomes **Blocked** on exit `1`; a user-supplied direction and reason produce **Proposal awaiting approval** on exit `3`; only a later explicit approval applies that exact token and reaches **Claimed** with a reservation id. A stale token that returns exit `3` replaces the proposal and requires approval again, a changed conflict set that returns exit `1` returns to **Blocked**, and malformed-token exit `5` is reported separately and never treated as approval. No path mints and spends in one turn. Every path the skill emits is lexically valid and repo-relative, **whether or not it exists yet**; the validator rejects an absolute path, a `..` escape, and a `.git` component; two hand-written blocks that overlap are reported as colliding with no ledger present; a `claim` that returns exit `6` is reported as busy rather than as clear, asserted to have invoked the verb exactly once with no shim-side retry loop; all four commands resolve the same validator. Also: `/sync check` runs drift's full phase-start comparison and is asserted to issue that comparison's four commands, not the `check` verb's proposed-path question; and `check` remains reachable for the proposed-path and proposed-edit questions it actually answers.

### Phase 15 — /plan:delegate integration  · status: todo

#### Work Order

**Goal:** In `hana`: a delegated phase claims its reservations before the first implementation dispatch and releases them at checkpoint, without anyone remembering to.

**Spec:** Claim **before the first implementation dispatch**, recording the phase's starting `HEAD` as the fingerprint baseline. The dispatcher invokes phase 14's shared `/sync claim --from-work-order` flow; it does not call `cargo-berth claim` directly or implement another proposal/token loop. Release through `/sync release` at the checkpoint boundary that already exists — which records `Outstanding { protected_tip }`, not disappearance (phase 4).

A phase whose Work Order has no `**Reservations:**` block will become a hard stop with an actionable message, not a silent skip — a phase that claims nothing is invisible to every other worktree, which is the exact decay this design exists to prevent. Implement the check here behind an explicit **advisory** coverage mode: before activation, a missing block reports that mandatory coverage is not active and permits the phase to continue, while a present block uses the full claim lifecycle. Phase 17 changes that named mode to **required** only after phase 16 finishes the backfill; required mode refuses a missing block before dispatch. This phase's dry run uses a Work Order that already carries a block, and its gate proves a Phase 16 Work Order remains dispatchable while coverage is advisory.

On a blocked neutral claim, `/plan:delegate` pauses before dispatch and enters phase 14's shared **Blocked** state. The `/sync` flow alone gathers the user's answer and reason, presents **Proposal awaiting approval**, and applies an explicitly approved token. Only **Claimed** may continue to implementation. A claim or release that exits `6` is contention rather than a verdict, and it arrives with phase 8's whole retry budget already spent inside the engine — one 10-second total deadline across all attempts, with exponential backoff from 50 ms inside it. Do not retry the invocation: stop with a busy-ledger message naming the command to run again, rather than dispatching as though the claim had succeeded.

**Files:**

- `~/.claude/commands/plan/delegate.md` — claim/release at the two boundaries. **Edit with the Write/Edit tool, never a shell write** — `~/.claude/commands` is a protected path and shell writes fail with `Operation not permitted`.

**Constraints from prior phases:** Phase 14's shared validator does the Work-Order resolution and its shared claim coordinator owns **Blocked → Proposal awaiting approval → Claimed** — `/plan:delegate` invokes that `/sync` entry point rather than parsing markdown or driving token application itself. Phase 4's release semantics mean checkpoint does not free the paths. Phase 3's provenance flags are what carry the plan, phase, coordination run, and starting `HEAD` into the claim. Compile nothing. Phase 3 owns `EditAuthorization` and its resolution, which phase 9b reordered — the session-keyed mapping first, then `CARGO_BERTH_RUN`, then the worktree marker file, failing closed to `Unidentified`. Read it; never re-derive it, and never pass a run id to `check`. Phase 3b's `claim --run` is the one provenance boundary where a run id is an argument; phase 9b's session mapping records that same value against the harness session — the same UUID-v7, not a second identifier — so a run that claims and a later check in that worktree agree on who is acting. `--plan` and `--phase` are additive optional flags that travel **together**; supplying one without the other is a usage error, not a partial record. `--head` records the commit the phase started from and is what phase 9's drift fingerprint later diffs against, so the dispatcher must pass the real starting `HEAD` rather than omitting it. A claim can succeed while reporting `marker_publication: unavailable`; phase 9b's session mapping is what carries identity through the phase in that state, rather than the marker file. **The claimed `ReservationId` has to survive the phase, and the session mapping is not where it survives.** Two facts about phase 9b's shipped mapping decide this. It holds exactly one reservation per harness session — a `BTreeMap` keyed by session id — so a second claim in the same session **replaces** the first rather than joining it. And no command reads it back out: `release` and `renew` take the reservation as a required positional argument, and nothing prints the mapping. The mapping is a disposable projection that answers one question for the engine — who is acting in this process — and it answers it well; it is not orchestration memory.

So `/plan:delegate` keeps its own record. Persist a named `ActivePhaseReservation` under the run's `SESSION_DIR` when **Claimed** returns — the reservation id, the coordination run, the phase it belongs to, and the starting `HEAD` — and read the release argument from there at checkpoint. `SESSION_DIR` is where that command already keeps durable run state and is what survives a compaction, a resume, or an orchestrator that lost its context mid-phase; a value held only in conversation does not, and a value held only in the mapping cannot be read back. Delete the record when the release succeeds. The mapping stays exactly what phase 9b built it for: the `PostToolUse` shim passes the harness session id and the engine resolves the acting reservation from it, with no shim reading the file and none supplying `--reservation`. **The pre-checkpoint check runs drift's full comparison.** The cheap delta answers only whether anything changed since the last observation; before releasing, the question is what changed against the phase-start baseline, which is phase 9's four-command comparison selected explicitly. The retry budget is phase 8's single 10-second total deadline across all attempts, not five attempts each paying the engine's internal lock wait. **Phase 5 changed what a marker-derived identity buys.** `EditAuthorization` is now source-preserving — `Environment(CoordinationRunId)`, `Marker { coordination_run_id, worktree_id }`, or `Unidentified` — and a `Marker` identity is honored only when replay shows that same run still holds an **active** reservation minted in that same worktree. An environment-supplied run is trusted as given. A stale marker left by a crashed run therefore no longer grants the holder's own exemption, which is exactly why phase 9b's session-keyed mapping is the primary path and `CARGO_BERTH_RUN` and the marker file are the fallbacks behind it. **The engine already owns the only retry budget, so a shim must not open a second one.** Phase 8 set the contract and phase 4's lock implements it: one 10-second total deadline measured across all attempts, with exponential backoff from 50 ms inside it. Exit `6` is returned only once that whole deadline is spent, so a caller that sees `6` has no budget left to retry with — a second-layer loop just multiplies a ten-second wait by however many times it runs and turns a busy ledger into a hang. Treat `6` as the named exhausted outcome it is: report that the tool was busy and nothing was decided, name the command to run again, and let the person decide. Do not write a retry loop around a verb, and do not assert one in an acceptance gate.

**Acceptance gate:** No `verify.sh`. A dry run over a real todo phase invokes the shared `/sync` flow, claims the right paths, records the run and reservation in phase 9b's session mapping, and releases at checkpoint leaving the reservation `Outstanding` and retiring that mapping; a forced collision pauses before dispatch and uses the same blocked/proposal/claimed transitions as interactive `/sync`, including stale-versus-malformed handling; a claim held off by a busy lock produces exactly one dispatcher invocation — the engine's own lock attempts stay inside the single 10-second deadline, no phase is dispatched, and the user receives an actionable message naming the command to run again — and the dispatcher is asserted never to reinvoke the verb; two phased claims made under **distinct** harness session ids each map to their own reservation and each `drift` resolves unambiguously, while a second claim made under the same session id is shown to replace the first in the mapping — which is why the dispatcher's own `ActivePhaseReservation` record, not the mapping, is what the release argument comes from; a checkpoint taken after the orchestrator's context is discarded still releases the right reservation by reading that record; and the pre-checkpoint check runs the full comparison against the recorded phase-start `HEAD`. While coverage mode is advisory, a Phase 16 Work Order with no `**Reservations:**` block reports the missing declaration but remains dispatchable; required-mode refusal is activated and tested in Phase 17.

### Phase 16 — Backfill 28 Work Orders  · status: todo

#### Work Order

**Goal:** In `hana`: every live Work Order in both plans declares its reservations, so the two plans can be compared before either runs.

**Spec:** 28 `todo` Work Orders need a `**Reservations:**` block: `docs/hana/tool-graph.md` (19) and `docs/hana_valence/arrangements.md` (9). **`done` phases are not touched.**

25 are generable from the existing `**Files:**` block in the same Work Order: take each backticked path, expand brace notation (`{lib,plugin}.rs`), classify as `file:` or `tree:`, and reduce to a minimal antichain. Paths are **repo-relative** — matching what is on disk, not absolute.

Three have no `**Files:**` block and must be authored by reading the phase: **Tool Graph 60, 69, 70**.

Grammar, R35 in `docs/berth-plan.md` (this plan):

```markdown
**Reservations:**

- file: `Cargo.toml`
- tree: `crates/hana/src/transport`
```

Do not widen a claim to swallow a whole crate to save effort — rolling `crates/hana_*` up to `crates` eliminates all useful concurrency. Claim at the lowest necessary root.

Once backfilled, **report every collision found** — the known one is Tool Graph 78 and Valence 27, which both name `crates/hana/src/main.rs` and both touch `crates/hana/src/input/`. Record the collisions; do not resolve them here.

Find them **offline, by comparing the 28 blocks against each other**, using phase 14's shared validator to parse each block and phase 3b's overlap rule to compare each pair. Do **not** run `cargo-berth check`: `check` tests one footprint against reservations that are actually live in the ledger, and no phase in either plan has claimed anything yet, so it reports no collision however badly two Work Orders overlap. The question this phase answers is whether two *plans* can run concurrently, which is a pairwise comparison of declarations, not a query against live state.

**Files:**

- `/Users/natemccoy/rust/hana/docs/hana/tool-graph.md` — 19 todo Work Orders.
- `/Users/natemccoy/rust/hana/docs/hana_valence/arrangements.md` — 9 todo Work Orders.

**Constraints from prior phases:** Phase 14's shared validator parses this grammar — the blocks must satisfy that one parser exactly, and this phase calls it rather than writing another. Phase 3b's antichain reduction defines "minimal", and its validation is **lexical**: a declared path need not exist on disk, because a Work Order routinely claims the file it is about to create. `Cargo.toml` and `Cargo.lock` are ordinary exclusive reservations — a Work Order naming one holds it for that phase's duration, and a second Work Order naming it collides. Expect that collision to be common and report it like any other. Compile nothing: this phase edits markdown in `hana` and runs the already-installed binary from phase 11 only if it runs anything at all.

**Acceptance gate:** No `verify.sh`. All 28 blocks parse through phase 14's validator; the pairwise comparison is exercised against all four overlap rules — a sibling whose name merely prefixes another does not collide, a `file:` and a `tree:` over the same spelling collide only where the tree contains the file, a path that does not exist yet participates, and two blocks differing only in case collide because this repository sets `core.ignoreCase`; no block rolls up above the lowest necessary root; `done` phases are byte-identical; the collision report is produced by pairwise comparison of the 28 blocks and names TG78/Valence27 and any others, including every manifest and lockfile collision.

### Phase 17 — End-to-end proof, then enforce  · status: todo

#### Work Order

**Goal:** In `hana`: two real worktrees prove the whole loop, and only then does the trunk gate start enforcing.

**Spec:** The end-to-end test is the gate on the gate.

**First, activate the coordination surfaces.** Phase 13 built all three shims and tested them by hand but deliberately left `/Users/natemccoy/rust/hana/.claude/settings.local.json` untouched, so nothing invokes them yet. Add the `hooks` key registering the `PreToolUse` shim for `Edit`/`Write`/`NotebookEdit`, the `Bash` incursion shim, and the `SessionStart` reconciliation shim, preserving the existing `permissions` and `outputStyle` values **byte for byte** — this file is the user's own settings and the only allowed change is the added key. Phase 15 also installed missing-`Reservations` detection in advisory mode so phase 16 could run before the backfill existed. Now that phase 16 has validated every live Work Order, change that named coverage mode to **required** and prove a missing block refuses dispatch. Until both activations land, the scenarios below do not test the deployed workflow.

Then create two real worktrees off `main` and prove, in this dependency order:

1. A collision **blocks** — worktree B's `Edit` into a path worktree A claimed is refused by the `PreToolUse` hook, naming A's branch and phase.
2. `CARGO_BERTH_BYPASS=1` **works when broken** — with `journal.ndjson` deliberately corrupted, editing still works and a bypassed merge succeeds.
3. A `Bash` write into a foreign claim surfaces as an **incursion**.
4. A `sequence` **holds** — start from the neutral exit-`1` block, take the user's `--after` answer and reason to obtain exit `3`, apply the exact token only after separate approval, then show both worktrees editing freely while B's merge to `main` is refused with A unmerged.
5. Landing in order **releases the edge from scenario 4** — A merges, B's hold clears only after B has A's `protected_tip` as an ancestor of its `HEAD` (R69), and a rebase + resnapshot satisfies it.
6. `--force` **lands and is visible** — create a fresh isolated edge, force the held merge, and show the board marking that edge bypassed with its reason and flagging the predecessor.

**Scenarios 4, 5, and 6 must record the ordering status at each step, not only whether the merge went through.** A merge outcome alone cannot tell a passing run from one that happened to permit for an unrelated reason, and the states the tool reports are exactly what a user reads when they are stuck. So the transcripts record: in scenario 4, the exact holding state before A lands, and again after A merges, where it must change to the distinct "the predecessor is on trunk but this worktree has not incorporated it" state rather than staying the same word; in scenario 5, that the edge reaches fulfilled only once B's current `HEAD` contains A's protected tip, and that it was `HEAD` ancestry that cleared it — the resnapshot updates B's own protected checkpoint and is a separate effect, so the transcript must not let one stand in for the other; and in scenario 6, that the bypass annotation on the board is a recorded fact about a merge that was forced through, kept visibly separate from the derived status of the edge itself, which never reads as satisfied.

**Scenarios 4 and 6 cannot prove refusal or force behavior while the trunk gate is observe-only, and scenario 5 must follow the edge created by scenario 4.** In observe mode the gate reports its decision and then permits the merge, so run the proof accordingly:

- Scenarios 1–3 run first against the observe-only gate. Record the gate's stated decision at each step, not merely the process outcome.
- Scenario 2 deliberately corrupts `journal.ndjson`. Run it against a **disposable coordination domain** — a scratch ledger the scenario creates and deletes — not the repo's real ledger. If it must use the real one, repair the journal and verify it reads clean before enforcement is enabled.
- Then flip `gate_mode` to `"enforce"` **temporarily** and run scenarios 4, 5, and 6 in that order. Scenarios 4 and 5 intentionally share one edge; scenario 6 must create its own. Flip straight back to `"observe"` if any of the three fails. The rollback is part of the proof, not a follow-up: a failed run must never leave the real repo enforcing a gate that has not been proven.

Only after all six pass, make `gate_mode = "enforce"` durable in `.claude/config/berth.toml`; if the temporary flip is still active, verify and retain it, otherwise perform the final flip then.

Then clean up: remove the test worktrees, and record in this plan's status line that the design is built.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/settings.local.json` — add the `hooks` key; `permissions` and `outputStyle` unchanged.
- `/Users/natemccoy/rust/hana/.claude/config/berth.toml` — `gate_mode = "enforce"`.
- `~/.claude/commands/plan/delegate.md` — change the named reservation-coverage mode from advisory to required after the backfill is verified.
- `/Users/natemccoy/rust/cargo-berth-init/docs/berth-plan.md` — status line: built. **The path is absolute deliberately**: this phase runs from the `hana` checkout, where a relative `docs/berth-plan.md` resolves to a different repository's file or to nothing at all.

**Constraints from prior phases:** Every prior phase is exercised here. Phase 8's observe-only default is what makes this safe to run against the real repo — do not flip it early, and do not leave it flipped after a failed scenario. Phase 12 established the ledger and the config; phase 13 built the three hook shims but left them unregistered, which is why this phase's first step is registering them. Phase 15 left reservation coverage advisory so phase 16 remained dispatchable; switch only that named policy to required after validating the backfill. Apart from that activation, this phase changes only the settings hook key and gate mode. Compile nothing: the binary was installed in phase 11 and its version and path are recorded there. Phase 13 specifies and tests the pre-edit shim for `Edit`, `Write`, **and `NotebookEdit`**; register all three here and cover `NotebookEdit` in the end-to-end scenario. The `SessionStart` reconciliation and alert shim must also be registered, or phase 5's durable orphan alerts and phase 8's deferred bypass report reach no one. Phase 14 owns the full **Blocked → Proposal awaiting approval → Claimed** sequence; scenario 4 must use it without abbreviating the proposal round trip. Phase 3 owns `EditAuthorization` and its resolution, which phase 9b reordered — the session-keyed mapping first, then `CARGO_BERTH_RUN`, then the worktree marker file, failing closed to `Unidentified`. Read it; never re-derive it, and never pass a run id to `check`. Phase 3b's `claim --run` is the one provenance boundary where a run id is an argument; phase 9b's session mapping records that same value against the harness session. **Phase 5 changed what a marker-derived identity buys.** `EditAuthorization` is now source-preserving — `Environment(CoordinationRunId)`, `Marker { coordination_run_id, worktree_id }`, or `Unidentified` — and a `Marker` identity is honored only when replay shows that same run still holds an **active** reservation minted in that same worktree. An environment-supplied run is trusted as given. A stale marker left by a crashed run therefore no longer grants the holder's own exemption, which is exactly why phase 9b's session-keyed mapping is the primary path and `CARGO_BERTH_RUN` and the marker file are the fallbacks behind it. **Commit the configuration before creating any linked worktree.** `.claude/config/berth.toml` is repository content resolved per worktree through `git rev-parse --show-toplevel`, not a single file the common git directory shares. A worktree created before that file is committed reads no configuration and every command run inside it reports an unreadable ledger, which is indistinguishable from a real engine fault until someone checks that worktree's own checkout. It cost real time during phase 8's smoke run. **Keep the gate mode consistent in every worktree that runs `integrate`.** Configuration is read from the invoking worktree, so temporarily switching to enforcing in one checkout leaves every other checkout still observing. Either set the mode in every worktree that invokes `integrate` during a scenario, or run those invocations only from the policy checkout, and say which. **The scratch-ledger scenario needs its own repository.** The ledger's location is fixed under the repository's common git directory, so no worktree can create and delete an isolated ledger while its real coordination state survives — corrupting the ledger to exercise recovery corrupts the only ledger there is. Run the corruption scenario in a separate scratch repository or clone, initialized with the same installed binary, and exercise corruption, the bypass marker, confirmed reinitialization, and `SessionStart` recovery there. The claim that all six scenarios run in the same two worktrees is wrong and must not be repeated. That scenario also proves the marker drain is idempotent: interrupt the import between the journal append and the marker delete, then run it again, and assert one journal record and no marker. **It also proves the drain is announced exactly once.** Phase 10b separates the set a board read just adopted from the durable bypass audit history, and phase 13's `SessionStart` shim reports the first and not the second. So the same scenario runs `SessionStart` twice against that scratch repository: the first run names the recovered marker set, the second is silent about it, and the bypass itself stays visible on the board's audit history across both without being re-announced.

**Acceptance gate:** No `verify.sh`. The `hooks` key is present, registers all three shims, and the rest of `settings.local.json` is byte-identical to its prior contents; reservation coverage is required, a Work Order with no `**Reservations:**` block now refuses before dispatch, and the previously completed Phase 16 advisory-mode proof remains recorded; a session started against a ledger carrying an orphaned-outstanding alert surfaces it; all six scenarios pass with transcripts recorded — five against two real worktrees, and the ledger-corruption scenario against its own separate scratch repository or clone per the constraint above, never against the only ledger there is, each transcript naming the `gate_mode` in force when it ran; scenario 4 records exit `1`, exit `3`, explicit token approval, and the resulting edge before scenario 5 clears that same edge; the scenario 4, 5, and 6 transcripts each record the reported ordering status at every step, showing scenario 4's hold changing to the incorporate-the-predecessor state once A lands, scenario 5 reaching fulfilled only on `HEAD` ancestry and distinguishing that from the resnapshot, and scenario 6 keeping the bypass annotation separate from the edge's own derived status; the journal used by scenario 2 is either disposable or verified clean afterwards; the scratch-repository scenario runs `SessionStart` twice after a pending bypass marker is drained and asserts the recovered set is reported on the first run, absent on the second, and that the bypass remains in the board's audit history throughout; the failed-scenario rollback to observe mode is tested; `gate_mode = "enforce"`; an out-of-order merge to `main` is then refused in a real terminal, and `CARGO_BERTH_BYPASS=1` still lands it. Also: one scenario drives a real incursion end to end — a write in worktree A lands inside worktree B's blocking reservation, the registered `PostToolUse` shim surfaces it to A, the board and the `SessionStart` shim surface it to B, and it is then answered with `resolve <reservation-id> --incursion <incident-id>`, the command phase 9b shipped, after which no incident remains outstanding while the answered one still appears in the recorded-answer audit as resolved. Deleting the test worktrees is not a resolution and does not satisfy this item. Cleanup at the end of the run releases the scenario reservations, settles the ordering edges, and answers every incident it opened, and the board proves it: nothing remains outstanding. Resolved incidents are not removed — replay retains each as `Resolved { resolution_event_id, resolved_at }`, and an audit trail that survives is the point, so assert that they stay in the resolved audit rather than asserting the journal forgot them.

---

## Design

Everything below is the design the phases implement, merged in from what was
`hana/docs/worktree-sync.md`. Work Order Specs cite it by section heading and by
finding id; it is part of this plan, not a separate document.

Read it as three layers. **Design** — this section — is the committed shape.
**Review findings** R1–R69 are three cycles of expert review that corrected it: a
later finding supersedes an earlier one where they conflict, and each supersession
is named in the finding's own title. **Decisions** D1–D8 are the eight the user
ruled on directly.

### Goal

Any number of worktrees, all off one trunk, each running its own phased plan. Before
an agent touches a tree, it can answer one question cheaply: **is it safe to work
here?** When the answer is no, the conflict surfaces to the user while it is still a
sentence, not after it has become a merge.

### Why the old board is gone

`SYNC.md` carried three things bundled. Two died with the merge.

| What it carried | After the merge |
| --- | --- |
| Cross-repo dependency plumbing — rev pins, `[patch]`, path repointing | Gone. One workspace, all path dependencies. |
| Merge-order gating across four branches in two repos, where neither session could observe the other | Gone. Worktrees of one repo share an object store and see each other's commits. |
| One legible picture for one person tracking parallel work | **Survives.** The only durable need. |

The board's real cost was that most of it was a hand-maintained cache of facts git
already knew — ahead/behind, what merged where, which commit carries what. Eight
protocol rules existed to keep that cache honest, and they held only as long as every
agent remembered rule 1. What git could *not* supply was intent, ownership, ordering
rationale, and permission. Those four are what this design keeps; everything else is
computed on demand and never stored.

### The ledger

#### Where it lives

`$(git rev-parse --git-common-dir)/cargo-berth/`.

Every linked worktree resolves `--git-common-dir` to the *same* main `.git`, so one
file is visible identically from all of them. It sits on no branch, so it never
merges and never diverges. It does not appear in `git status`. Verified on a scratch
repo: a file written under the main checkout's `.git/cargo-berth/` reads back unchanged
from a linked worktree, and `git status` there is clean.

Cost of this choice: `.git` is not backed up and does not travel to another clone.
The branches themselves are unaffected — losing the ledger loses coordination state,
not work.

#### Truth is the journal; the cache is disposable

Two files:

- **`journal.ndjson`** — append-only, one JSON object per line. This is the record.
- **`reservations.json`** — the live claim set, a projection of the journal. Rebuildable
  by replay, safe to delete at any time.

This is deliberate. The old board needed rules 5 and 6 — commit after every edit,
never `checkout`/`reset`/`restore` — because a whole-file write that dropped rows left
behind something that looked exactly like a correct board. Appends under `PIPE_BUF`
with `O_APPEND` cannot interleave destructively, so two sessions claiming at the same
moment cannot clobber each other. The failure mode is designed out rather than ruled
against.

#### The unit: lowest necessary tree root

A claim is a **repo-relative path** — the shallowest directory that covers what the
phase will touch. `crates/hana_valence` when a phase owns the crate;
`crates/hana/src/transport` when it owns one module.

The conflict rule is one line:

> Two claims conflict iff one path is a prefix of the other **and** the branches differ.

No crate-graph knowledge, no configuration, works at any depth. Same-branch claims
never conflict — a delegate fan-out running several agents on one branch is one actor.

A claim record:

```json
{"op":"claim","path":"crates/hana_valence","branch":"feature/valence",
 "worktree":"/Users/natemccoy/rust/hana_valence","plan":"docs/hana_valence/arrangements.md",
 "phase":24,"why":"arrangement providers","at":"2026-08-23T11:02:00Z"}
```

#### Claims are derived, not hand-written

Every Work Order produced by `/plan:to_phased_plan` already carries a `**Files:**`
section listing the exact paths that phase touches. That is the footprint, already
written, already reviewed. The `/sync` skill reads it (the tool never
parses markdown) and rolls the file list up to its shallowest covering directories.
**Measured 2026-08-23: `**Files:**` blocks on disk record repo-relative paths**, not
absolute ones — no prefix stripping is needed, and a claim path is already in the form
the ledger stores.

Nothing new for the user to maintain, and a claim cannot drift from the plan it came
from without the plan changing first.

### The check

Three tiers. The third is what keeps the system honest.

| Tier | Question | Source | Result |
| --- | --- | --- | --- |
| **Overlap** | Is another branch inside my tree? | ledger | **Block** — surface to the user |
| **Dependency** | Does my crate depend on, or is it depended on by, a crate another branch holds? | `cargo metadata` | **Warn** |
| **Drift** | Did I touch files I never claimed? | `git diff --name-only $(git merge-base main HEAD)..HEAD` | **Widen the claim** |

**Tier 2 is the one that catches real merge pain.** Textual overlap is the rare
failure in a monorepo of disjoint crates. The common one is branch A reshaping
`hana_kana`'s API while branch B builds on it: zero conflicting lines, broken on
merge. The dependency graph is free from `cargo metadata` and turns that into a
warning at claim time.

**Tier 3 is the rot detector.** Declared intent is checked continuously against
observed diff. A file touched but never claimed means the claim is too narrow and the
work is invisible to the other worktrees — the exact condition that made the old board
decay, now detected instead of legislated against. It costs one `git diff` to compute
and is never stored.

`sync board` additionally reports ahead/behind against `main` per live branch,
computed live. That answers "what moved under me while I was away" without anyone
recording it.

#### The root manifest exception

`Cargo.toml`, `Cargo.lock`, and `.claude/config/*` are shared by construction — every
branch touches them, so exclusive claims on them would block everything. They are
**announced, not claimed**: the check reports "3 branches will touch root
`Cargo.toml`" as information and does not block. `Cargo.lock` remains
regenerate-never-merge.

#### Release, staleness, override

- **Release** at phase checkpoint, the same boundary that already exists in
  `/plan:delegate`.
- **Stale claims are flagged, never auto-removed.** A claim is stale when its worktree
  path no longer exists (definite) or its branch has no commit in seven days
  (probable). Silent removal would produce a false all-clear, which is worse than a
  stale row someone has to read.
- **Override** is required or the hook becomes a wall. `sync override <path> --why`
  proceeds and writes the reason to the journal.
- **A missing ledger fails open.** Everything is safe when `reservations.json` does
  not exist. A coordination tool that bricks editing when its own state file is gone
  is worse than no tool.

### Answering an overlap

A tier-1 block is a question, not a verdict. It has four answers. Three of them permit
an overlap that would otherwise be refused, so all three are **recorded inside the same
locked `claim`/`widen` transaction that discovered the overlap** (R53) and all three
**require user approval** through R42's `permissionDecision: "ask"` (R54). Only
`rescope` needs no authorization, because it leaves nothing overlapping.

| Answer | Means | Editing | Integration |
| --- | --- | --- | --- |
| `rescope` | Narrow or split the claims so the overlap stops existing | Both proceed | Unconstrained |
| `sequence` | A lands before B | Both proceed | B held until A is in trunk |
| `defer` | Overlap accepted; order not decided yet | Both proceed | **Both held** until an order is declared |
| `override` | Overlap accepted; no order will be enforced | Both proceed | Unconstrained; reason journalled |

`sequence` is the expected answer for two plans that genuinely need the same file.
`defer` is for when the right order depends on something not yet known: it buys the
same "stop asking me" as `override` without giving up the guarantee, because neither
reservation can reach trunk until someone says which goes first. That makes deferring
cheap without making it the quiet path to an unordered merge (R56).

**An answer authorizes exactly what it was shown.** It is bound to the two reservation
ids *and* the normalized overlap antichain at the moment it was given, plus both
reservation generations. The hook suppresses only paths covered by that recorded set.
A later widen recomputes the intersection and re-blocks anything the answer never
covered; an answer is never transitive to a third reservation, and a new reservation id
never inherits one (R55).

#### Sequencing

`claim --after <blocker> --why` and `claim --before <blocker> --why` record
`ConflictAuthorization::Sequence` as part of the acquisition itself. `sync sequence`
exists only to change an answer already given. Recording an edge does four things:

- **Both worktrees proceed immediately** on the paths the answer covered.
- **The successor is held at integration until it has actually incorporated the
  predecessor** — not merely until the predecessor lands. The gate is the predecessor's
  journaled `Outstanding.protected_tip`: first that it is an ancestor of trunk, then
  that it is an ancestor of the successor's `HEAD`. A predecessor still `Active` has no
  protected tip and holds unconditionally. The successor's live branch tip is never the
  subject, and the claim-time `HeadSnapshot` never is either (R57, R69). A retention ref
  `refs/cargo-berth/reservations/<id>` keeps that commit reachable after the branch or the
  worktree is gone.
- **Cycles are rejected at declaration, under R43's descriptor-held mutation lock** —
  replay, validate both endpoints, run the cycle check, append, sync, publish. Without
  the lock, two worktrees can each replay an acyclic graph and append `A → B` and
  `B → A` (R58).
- **Edge status is derived, never stored as a terminal fact.** An edge is pending, met,
  or cancelled purely as a function of the predecessor's lifecycle and *current* trunk,
  revalidated on every check per R41. A trunk rewrite that removes the predecessor
  re-holds the successor (R60).

**Cost.** Not free, but bounded (R67). Adjacency rebuilds during replay in `O(J+V+E)`;
the cycle check is a DFS in `O(V+E)`; readiness groups by predecessor, so a board read
costs one `git worktree list --porcelain` plus at most `P` ancestor checks
(≈`0.01·P` s), and an integration check costs only the successor's `d` prerequisites.
Duplicate edges are rejected and R4's limits extend to `V` and `E`. None of this runs
on the hook path, which reads only the generation-validated projection (R31).

**Ordering is conflict-time state.** A Work Order declares its reservations, never its
expected order — the relationship exists only while two reservations are live, so
putting it in the plan would duplicate something that is not knowable when the plan is
written.

#### Self-healing

Edges resolve themselves as reservations reach their endings. What they never do is
resolve themselves by *assuming* an ending.

- **A missing worktree is not an abandoned reservation** (R59). Removing a worktree
  does not delete its branch or its commits, and absence can equally mean a prune not
  yet run, a lock, a moved directory, or broken admin linkage. Liveness is typed —
  `Live | Unavailable | OrphanCandidate | Orphaned | Unknown` — from
  `git worktree list --porcelain` plus R39's opaque identity check. Everything except
  `Live` **retains** the reservation's scopes and edges. A successor is freed
  automatically only on proven integration evidence; otherwise the edge waits for a
  user-approved retirement. This is the same rule the design already applies to stale
  claims: flagged, never auto-removed.
- **Reconciliation runs on the paths that consume the state**, not only on board read
  (R62): at SessionStart, before every stateful `/sync` verb, and before every
  checkpoint and integration. The edit hook keeps its fast path — it reconciles only
  when the cache says it should block, then retries the decision once.
- **Incident edges are evaluated independently** (R61). With `A → B → C`, losing `B`
  terminates `A → B`, resolves `B → C` from `B`'s stored evidence, and **never
  synthesizes `A → C`**. `C` waits on `A` only if that edge was declared.
- **Orphaned work gets a durable alert, not a message** (R63). A reservation left
  `Outstanding` when its worktree disappears raises an `OrphanedOutstanding` alert that
  persists — shown at SessionStart, from every `/sync` verb, and whenever a hook
  evaluates it or one of its successors — until the user records recovery, integration,
  or approved abandonment. It reports the reservation id, protected tip, branch-ref
  status, object availability, and one of `RecoverableFromBranch`,
  `RecoverableFromProtectedTip`, or `CommitUnavailable`. "Commits are lost" is a
  conclusion it has to earn.
- **The reservation's terminal record is the sole authority for its edges** (R65).
  Reconciliation appends one record under the lock and derives every incident-edge
  outcome from it at replay; per-edge records are audit observations, regenerated
  idempotently. A crash mid-reconciliation therefore cannot leave a valid prefix that
  frees a successor before the decision that freed it is durable.
- **Ledger loss fails open for editing and closed for integration** (R64). R3 is right
  that a missing ledger must not brick editing — but a lost journal also erases a
  user-approved merge order, and proceeding past that is not the same risk. An absent,
  corrupt, or unknown-epoch journal blocks integration until the user confirms pending
  orders were reviewed or reconstructed.
- **The board renders constraints, not a queue** (R66). A DAG is a partial order; a
  numbered list would invent ordering between unrelated reservations. It shows the ready
  set, each held reservation with its named predecessors and covered paths, unresolved
  overlaps, and unconstrained live reservations — and with no edges recorded it says
  "no integration order declared" rather than showing an empty queue that reads as an
  all-clear.


### Enforcement

Hook for what must never be forgotten; skill for what needs judgment.

#### The hook

`PreToolUse` on `Edit` / `Write` / `NotebookEdit`. It reads the ledger, matches the
target path, and **blocks only on tier-1 foreign-branch overlap**. When clean it says
nothing — zero noise on the overwhelming majority of edits. The block message names
the holding branch, plan, phase, and reason, which is enough to act on without
opening anything.

Tiers 2 and 3 never block. They are advisory and belong in the skill, because a
dependency warning is the start of a conversation about merge strategy, not a
stop sign.

**Known gap: Bash-mediated writes.** Auto mode directs file edits through `sed`,
heredocs, and short scripts, which a `PreToolUse` hook on `Edit`/`Write` does not
see. Parsing arbitrary shell for write targets is not reliable enough to depend on.
This is covered instead by tier 3 — anything that slips through appears as drift at
the next check, and the claim gets widened. Stated plainly rather than papered over:
hook coverage is partial, and tier 3 is what makes that acceptable.

An optional best-effort `Bash` hook could match obvious write forms (`>`, `>>`,
`sed -i`, `tee`) before extracting paths. Deferred — it buys partial coverage of a
gap tier 3 already closes.

**Outside Claude Code, tier 3 runs at commit time.** `init` installs a `post-commit`
hook that runs the drift comparison and reports an incursion, so someone with no
agent harness still gets an automatic check. It can only warn: git discards a
`post-commit` hook's exit status, and a warning after the commit is the most this
can do — blocking a commit because an edit strayed would trap the person mid-work
for a condition the tool cannot repair for them.

#### The trunk gate

The edit hook governs `Edit`/`Write`; nothing in it governs a merge. D1 leaves `Bash`
unconstrained, so `git merge`, `git rebase`, and `git update-ref` can move
`refs/heads/main` without any check running. An ordering edge is therefore only as real
as whatever enforces it at the ref (D8, resolved 2026-08-23).

A **`reference-transaction` hook** in the common git directory is that enforcement. It
fires on every update to `refs/heads/main` from any source — a terminal, an agent
through `Bash`, a slash command, a rebase — because it sits below the tool layer.
`--no-verify` does not skip it; that flag only covers the commit hooks. One hook in the
common directory covers every worktree.

Its rule is narrow. It denies only when a live reservation with an **unsatisfied
predecessor** would newly enter trunk. No edges, or nothing pending, means silence — the
overwhelming majority of merges never observe it. A denial names the blocking
reservation, its plan and phase, the covered paths, and the exact command to proceed.

This gate binds the user exactly as it binds an agent. That is the point, and it is why
the release valve is designed rather than discovered.

##### The release valve

Being unbypassable and failing closed (R64) is a trap: a corrupt ledger would block every
merge with no way out. Two escapes, at different levels, prevent that.

**`sync integrate --force --why "<reason>"`** — the ordinary escape. Mints a one-use
permit that the next `main` update consumes, journalled with actor, time, reason, and the
edges it skipped. This is the deliberate "we accept the harder merge" decision, and it
costs one command.

**`CARGO_BERTH_BYPASS=1 git merge …`** — the last resort, evaluated by the hook **before it
reads the ledger, the projection, or anything else**. A corrupt journal, an unreadable
lock, a hook that times out, or a bug in the gate itself can never leave anyone stuck. It
is journalled when the journal is writable and reported at next SessionStart when it was
not. Deliberately awkward to type, so it is not reached for by habit.

A hook timeout denies and names the bypass in the denial. Denying is safe precisely
because the bypass always works.

##### A bypass is recorded, not forgiven

The skipped edge stays on the board, marked bypassed with its reason and date, and the
predecessor is flagged: *ordered after work that already landed; expect conflicts*. The
consequence of the choice stays visible instead of being tidied away. This is the same
principle as R59 — the system reports what happened; it does not conclude on the user's
behalf that it was fine.

**Bypassing is not the same as changing the decision.** `--force` says "this once" and
leaves the edge standing. Converting the answer to `override` removes the edge and means
"not anymore." Keeping them distinct is what lets the board still mean something months
later.

#### The skill

`/sync` with these verbs:

| Verb | Does |
| --- | --- |
| `board` | The picture: live claims, holders, ahead/behind vs `main`, stale flags |
| `check` | All three tiers for a proposed footprint |
| `claim` | Explicit validated paths (the skill resolves a Work Order to paths, not the tool); `--before`/`--after`/`--defer`/`--override <blocker> --why` answers an overlap in the same transaction |
| `release` | At checkpoint |
| `sequence` | `<first> <then> --why` — change an ordering answer already given |
| `integrate` | The authoritative path to trunk: reconcile, check every incoming edge, then update `main`; `--force --why` mints a one-use permit past a held edge |

No mandatory emit ritual. The old board required a four-part emit at every phase
boundary and at every checkpoint; that was compensation for state nobody could
observe. Here the state is one command away from any worktree, so it is pulled when
wanted rather than pushed on a schedule.

### Where the code lives

The engine is **`cargo-berth`**, a new member of the `cargo-liner` workspace
(`~/rust/cargo-liner`, `github.com/natepiano/cargo-liner`) alongside `cargo-mend`,
`cargo-port`, and `cargo-tile`. A berth is the place assigned to one ship for exclusive
occupancy, allocated in advance — which is what a claim is.

It does not live in the hana workspace. Three constraints rule that out:

- **It must run when hana does not compile.** Mid-refactor is exactly when two worktrees
  collide. A coordination tool that needs a successful build of the thing it coordinates
  is circular.
- **The edit hook must be instant** (R31), so it calls an installed binary, never
  `cargo run` — living in the workspace buys nothing.
- **The `reference-transaction` hook runs inside git**, with a minimal environment and no
  cargo. It invokes `cargo-berth` as a plain binary on `PATH`; the `cargo-` prefix is
  irrelevant there.

`repo-split-and-publishing-mirror.md` also publishes and mirrors the hana workspace, and dev
coordination tooling does not belong in that surface.

`cargo-liner` supplies what this needs anyway: `cargo_metadata` is already a workspace
dependency (tier 2), `tui_pane` is the framework the board should be built on, and the
strict lint set and per-crate release cadence come with membership.

#### The split

Almost none of the hana-specific part is Rust.

| Where | What |
| --- | --- |
| `cargo-berth` | The whole engine, hana-blind: ledger, journal, claims, edges, cycle detection, the ref gate, the board. Its interface is paths and reservation ids. |
| `.claude/config/berth.toml` (in hana) | The repo's dialect: trunk branch, the announce-not-claim list, R4's limits. Sits beside `mirror.toml` and `release.toml`. |
| `~/.claude/` | The Claude Code integration: the `/sync` skill, thin `PreToolUse`/`PostToolUse` shims that shell out to `cargo-berth`, and `/plan:delegate` claiming at dispatch and releasing at checkpoint. |
| Plan docs | The 28 Work Orders' `**Reservations:**` blocks. Content, not code. |

**This moves `--from-work-order` out of the tool.** Parsing `**Reservations:**` from a
plan document is Claude-workflow territory: the skill extracts the paths and calls
`cargo-berth claim <paths>`. The tool still validates them — exist, normalize, reduce to
an antichain, check overlap under the lock — it just never reads markdown. That keeps
`cargo-berth` general enough to publish, and lets the plan-doc format change without
touching a released crate.

#### The README is a deliverable

`cargo-berth` publishes to crates.io alongside its siblings, so it ships with a README
written for someone who has never heard of hana, `/plan:delegate`, or Claude Code. It is
part of v1, not a follow-up.

What it has to cover:

- **The six commands, in order of a first use** — `cargo install cargo-berth`;
  `cargo berth init` (creates the ledger in `.git`, installs the trunk hook, writes a
  default config); `cargo berth claim <paths>`; `cargo berth board`;
  `cargo berth integrate`; `cargo berth release`.
- **What a collision looks like**, with real output: which branch holds what, and the four
  answers offered.
- **The honest limitation, stated plainly rather than buried.** The trunk gate is a git
  hook, so merge ordering is genuinely enforced for anybody with no discipline required.
  Editing is different: what makes it non-decaying here is a Claude Code `PreToolUse`
  hook that blocks the write itself, and that is our integration, not part of the tool.
  A general user gets a commit-time drift check — the same git hook family comparing
  changed files against the claim — which is automatic but later than blocking the
  keystroke. Say so. A coordination tool that oversells its enforcement is the failure
  mode this whole design exists to avoid.
- **The config file**, field by field.
- **What it deliberately does not do** — it does not choose the merge order, does not
  track phases, does not span repositories.

The `cargo-liner` root README also gains a `cargo-berth` row in its member list.

### Delivery shape

The work spans two repositories and is built by two agents who never read each other's
code. This document is the single source for both, so it has to carry everything each
side needs — the engine spec and R1–R69 for one, the wiring for the other.

| Track | Repo | Built by | Produces |
| --- | --- | --- | --- |
| **A — engine** | `~/rust/cargo-liner` | Its own agent, in that repo | `crates/cargo-berth`: ledger, journal, claims, edges, cycle detection, ref gate, board TUI, README, tests |
| **B — wiring** | `~/rust/hana` | An agent here | `cargo-berth init`, hook shims and their `settings.json` entries, the `/sync` skill, `/plan:delegate` claim/release, `.claude/config/berth.toml`, 28 Work Order backfills |

Track A needs no hana knowledge at all — that is the test of whether the split in
`### The split` is real. If a phase in track A has to explain a Work Order, the boundary
is wrong.

#### Freeze the interface first

The two tracks are only serial if track B has to wait for a working binary. It does not,
provided **phase 1 of track A freezes the command surface before anything is
implemented**: every verb, its arguments, its exit codes, and its machine-readable
output. Track B writes hooks and the skill against that contract while the engine is
still being built, and finds out at integration whether both sides read it the same way.

That contract is the deliverable of phase 1, written into this document. Changing it
afterward is a decision, not a refinement.

One such decision was taken: phase 2 adds `resolve` and `renew` to the seven verbs phase 1
froze, because four later phases assume a user can act on a stranded reservation and none of
the original seven does. The surface is still frozen before track B writes against it — track B
starts at phase 12 — so the guarantee this section describes is intact.

#### How hana gets the binary

`cargo install --path ~/rust/cargo-liner/crates/cargo-berth` during development. Nothing
publishes to crates.io until the whole loop works here — a published version is a promise
about an interface, and the interface is what we are still learning.

#### The trunk gate ships observe-only

An unproven `reference-transaction` hook can refuse every merge to `main`, including the
user's own from their own terminal. It installs in **observe-only** mode: it evaluates
every update, logs what it *would* have denied and why, and permits everything. It flips
to enforcing only after it has been right on real merges — including at least one real
sequenced pair.

That is not a soft launch for its own sake. Enforcement is the one part of this design
that can block work it was never meant to block, and the release valve is only reachable
if the person hitting the wall knows it exists. Watching it be right first is cheaper
than being trapped once.

#### Bootstrap ordering

We will be building the worktree coordinator inside a worktree, and installing its gate
into the repository it coordinates. Two consequences the phase order has to respect:

- **`cargo-berth` cannot coordinate its own construction.** Track A and track B are
  coordinated the way everything is today — one worktree, commits as work lands.
- **The gate installs in hana last**, after track B's end-to-end test passes. The
  end-to-end test is the real gate on the gate: two actual worktrees, a real collision
  that blocks, a real `sequence` that holds the successor, a real `--force` that lands
  anyway and shows up on the board as bypassed.

### A third query it answers for free

"Is crate X cold enough to publish?" falls straight out of the live claim set. Useful
for `repo-split-and-publishing-mirror.md` Phase 3 — publishing `hana_rigging` while a
branch is mid-rewrite of it is exactly the mistake this prevents.

### What this does not do

- It does not choose the merge order. It reports that two branches collide and then
  records whatever order you decide; deciding it stays a conversation with the user.
- It does not track plan phases or gates. Plans own their own phase numbering, as
  before.
- It does not span repositories. Cross-repo work goes through published versions, per
  `~/.claude/commands/worktree_fit.md`.

### Decisions

1. **Ledger in `.git/cargo-berth/`**, not a separate repo. One shared location for all
   worktrees at zero protocol cost; accepts loss of backup.
2. **Append-only journal is truth, `reservations.json` is a rebuildable cache.**
   Designs out the whole-file-clobber failure the old board needed two rules to guard.
3. **Path-prefix overlap is the conflict rule.** One line, any granularity, no config.
4. **Hook plus skill** (2026-08-23). The hook covers hard overlap and cannot be
   forgotten; the skill covers the board and the advisory tiers. Skill-only was
   rejected as the same discipline dependency that made `SYNC.md` decay; hook-only
   leaves the dependency warnings nowhere to land.
5. **An overlap has four answers, and one of them is an order** (2026-08-23, revised
   by cycle 3). Block-or-override was too blunt: two plans that legitimately need the
   same file had no way to say "TG78 lands first" and get on with it. `sequence` records
   the direction, unblocks both worktrees, and is enforced only where it matters — at
   integration. `defer` buys the same quiet without giving up the guarantee, by holding
   both. All three permissive answers ride the locked acquisition transaction and
   require user approval; an agent cannot answer its own block. Edge status is derived
   from reservation lifecycle rather than journalled as fact, so a trunk rewrite or a
   vanished worktree cannot leave a stale authorization standing.
6. **The engine is `cargo-berth`, outside the hana workspace** (2026-08-23). It has to run
   when hana does not build, the hooks call an installed binary either way, and the git
   hook has no cargo in its environment — so workspace membership offers nothing and
   costs publishing surface. `cargo-liner` already carries `cargo_metadata`, `tui_pane`,
   and a release cadence. The hana-specific part is config plus Claude Code integration,
   not Rust.

---

## Review findings — cycle 1

Team review, `strengthen` posture, 4 lenses. No premise-challenge: the common-directory
ledger can achieve the intent. Everything below is in-intent strengthening.

### Auto-recorded (accepted; single correct outcome)

**R1 — Reservation acquisition must be serialized by a lock. (critical)**
Atomic append placement does not serialize the transaction `read claims → check conflict →
append → publish cache`. Two sessions can read the same projection, both pass the check,
and both append conflicting claims. `O_APPEND` governs each write's file position, not the
surrounding transaction. Take one advisory lock in `cargo-berth/` for `claim`, `release`,
`override`, and cache rebuild; while held, replay, check overlap, append and `fsync` the
journal, then publish the cache by temp-file `fsync` + atomic rename. Store the journal
sequence or byte offset in the cache so the hook can detect a snapshot that is behind.
The lock is taken only at claim-lifecycle boundaries, never on every edit.

**R2 — The `PIPE_BUF` rationale is wrong and must be removed. (important)**
`getconf PIPE_BUF /tmp` is 512 on this machine, and POSIX applies that non-interleaving
threshold to pipes and FIFOs, not regular files. The sample record is already 230 bytes and
`worktree`, `plan`, and `why` are unbounded. Replace the claim in `## Truth is the journal`
with: one append routine under the mutation lock that handles short writes, a documented
maximum encoded record size, and rejection of oversized fields. Replay may discard exactly
one unterminated final record under the lock; a malformed interior record reports corruption
and stops mutation.

**R3 — A missing cache must not read as "safe". (critical)**
`reservations.json` is described as "safe to delete at any time" and its absence as
"everything is safe". Deleting only the cache leaves a journal full of live claims while the
hook allows every edit. Define four states instead: journal present + cache absent/behind →
rebuild before deciding; cache present without journal, invalid schema, or mismatched repo
identity → report corruption and deny until repaired; both absent → editing stays fail-open
but reports **"coordination inactive"**, never "safe", surfaced at session start and from
every `/sync` verb; require `sync init` in each new common directory. A fresh clone is a
separate coordination domain and cannot see an older clone's claims — say so.

**R4 — Hook exit-code matrix must be specified. (important)**
Claude Code command hooks treat exit 2 as blocking; exit 1, a missing executable, invalid
output, or a timeout all *allow* the call. So a parser exception silently disables the guard,
while converting every anomaly to exit 2 denies all edits. Specify and test: valid
non-overlap allows; overlap denies with structured output; uninitialized coordination allows
with a visible warning; stale cache rebuilds; corrupt journal/cache, unsupported schema, and
internal errors exit 2 naming one repair command. Bound record fields, cache size, live-claim
count, and parse time, and use an internal deadline shorter than the hook timeout so a denial
can be issued before Claude Code kills the script.

**R5 — Conflict comparison is path-component ancestry, not string prefix. (important)**
As written, "one path is a prefix of the other" falsely collides `crates/hana` with
`crates/hana_animation`. Compare normalized path components, respecting the filesystem's case
behavior.

**R6 — Branch is not a stable ownership or liveness identity. (important)**
Branches are renamed, deleted after merge, and reused; worktrees move or become locked. Seven
days without a commit can describe active uncommitted work, and a recent unrelated commit can
make an abandoned claim look current. Give each claim a stable id plus an explicit
coordination-run/worktree identity, and keep branch for display only. Renew claims at phase
checks rather than inferring liveness from branch commit time. Report branch-missing,
merged-to-trunk, worktree-moved, worktree-locked, and renewal-expired as distinct states, and
add `sync release --stale <claim-id> --why …` under user confirmation, keeping the
no-auto-removal rule.

**R7 — Same-branch fan-out needs a condition. (important)**
"Same-branch claims never conflict" suppresses overlap checking among all `/plan:delegate`
fan-out writers even when the dispatcher has not assigned disjoint files. Permit the shared
claim only when the orchestrator owns file assignment or serializes writers.

**R8 — Tier 3's diff command is the wrong range. (critical, mechanism half)**
`git diff --name-only $(git merge-base main HEAD)..HEAD` sees only committed work, which
excludes exactly the staged/unstaged/untracked state an agent is in mid-phase; and after a
checkpoint it includes every earlier phase since the branch diverged, so released paths get
re-flagged. Record the phase's starting `HEAD` at claim time and compare
`<phase-start> → working tree` plus untracked files.

### Proposed user decisions

**D1 — How do Bash-mediated writes get covered? (critical)**
Tier 3 runs *after* the write, so widening a claim cannot stop a second worktree that is
already editing. Given that this environment actively directs agents to edit through
`sed`/heredocs, the realistic worst case is two branches modifying a reserved file with no
hook ever firing, discovered at integration. The reviewer's proposal is a declaring wrapper —
`sync exec --paths … -- <command>` — with undeclared commands allowed only when provably
read-only, holding that matching a few redirection spellings is insufficient. That is a real
behavior change to how every Bash edit is issued. Alternatives: accept the gap and lean on
`/plan:delegate` integration (D2); or restrict the auto-mode Bash-edit preference in hana.

**D2 — Should `/plan:delegate` own the reservation lifecycle? (critical)**
The doc puts claim/release at phase boundaries but `/plan:delegate`'s checkpoint contract
contains no claim, drift check, or release, and `/sync` is otherwise pulled only "when
wanted". Proposal: claim before implementation dispatch, record the phase's starting `HEAD`,
check drift immediately before checkpoint, stop on newly discovered foreign overlap, release
only after checkpoint succeeds. This edits a shared workflow skill, not just this design.

**D3 — Root manifests: announce, or reserve for short windows? (important)**
Tool Graph phase 64 and Valence phases 24 and 32 all name root `Cargo.toml`/`Cargo.lock`, so
the announce-don't-block exception disables tier 1 precisely for the files most likely to
collide. "Regenerate-never-merge" is an integration procedure, not edit-time exclusion, and it
does not protect `Cargo.toml` or config files at all. Proposal: reserve root files
individually for short mutation windows — acquired immediately before a manifest edit or a
lockfile-generating cargo command, released after checkpoint — and apply ordinary
component-level claims inside `.claude/config/` rather than exempting the directory.

**D4 — Work Orders need a machine-readable `Reservation roots` field. (important)**
The claim-derivation story does not survive contact with the real docs: Tool Graph phases 60,
69, and 70 have **no** `**Files:**` section at all, and the sections that exist are
repo-relative, contain brace expressions, and carry prose like "`crates/hana_animation` tests"
and "All test/example/README/CHANGELOG files named in Delegation Context". `/plan:delegate`
also treats Files as predicted scope, not permission to edit. Proposal: add a separate
`Reservation roots` field of normalized literal files/directories and backfill it across both
plans before enabling derivation, with `sync claim` rejecting missing, empty, ambiguous,
outside-repo, `..`, or malformed entries and echoing derived roots before committing.

### Auto-recorded, cycle 1 continued (correctness lens; R1–R4 independently confirmed)

**R9 — Rollup must be a minimal component antichain, not one ancestor per phase. (critical)**
"Roll them up to their shallowest covering directories" reduces a multi-crate phase such as
Valence Phase 25 to `crates`, which eliminates all useful concurrency; a phase that also names
root files collapses to the repository root. Reduce *independently declared* paths to a minimal
antichain instead of taking a single least common ancestor across the phase.

**R10 — Claims need an explicit `Exact` vs `Tree` scope. (important)**
The design does not encode whether a claimed path denotes one file or a whole subtree. Without
it, `foo.rs` and `foo.rs.bak` confuse, and directory-vs-file comparison is undefined. Add
validated repo-relative paths with an explicit scope; reject empty paths, `.git`, post-ingestion
absolute paths, and any `..` escape. Conflict when exact paths are equal or a `Tree` path is a
component ancestor of the other claim. Specification tests: `crates/hana` vs `crates/hana_kana`,
file vs file-sibling, directory vs file, exact equality, nonexistent-future path.

**R11 — Owner identity comes from the worktree, not the branch. (important; sharpens R6)**
Derive a stable `owner_id` from the worktree's administrative directory and give each
reservation set a `claim_id`; branch, path, and `HEAD` are display metadata. Same-*worktree*
fan-out is one actor; **distinct worktrees conflict even when their branch labels match**, which
is the correct form of the rule the doc states as "same-branch claims never conflict". Detached
`HEAD` has no branch name at all.

**R12 — Release, override, and widen need defined event schemas. (important)**
Only a `claim` record is specified. Replay cannot currently determine whether a release targets
one of several same-owner claims, what a duplicate claim does, what releasing an unknown id
means, or how long an override lasts. Define versioned `create`/`widen`/`release`/`override`/
`retire-orphan` events; releases name a `claim_id` and unknown ids error; overrides identify the
blocked claim ids, covered paths, reason, and lifetime. Add a user-approved orphan-retirement
verb — the doc flags a deleted worktree as stale but gives no verb that resolves it.

**R13 — Drift widening must re-run tier 1 under the lock. (important)**
Unconditional "widen the claim" can silently create a foreign overlap. A widen that would
overlap another owner reports a collision instead of widening.

**R14 — Split active-phase drift from outstanding branch divergence. (critical; extends R8)**
Two computations, not one: (a) active-phase drift from the claim's starting `HEAD` plus staged,
unstaged, deleted, renamed, and untracked paths; (b) outstanding divergence from the current
merge base to the branch tip. When `main` advances, intersect trunk-side changes since the stored
trunk point against active and outstanding claims and their dependency packages, and recompute
after a rebase. Ahead/behind counts alone never identify *which paths* moved under a claim.

### Proposed user decisions, cycle 1 continued

**D5 — Must a claim be mandatory before editing? (critical)**
The hook "blocks only on tier-1 foreign-branch overlap", so an agent holding **no claim at all**
may freely edit any unclaimed path, and a later claimant sees no ledger conflict — the ledger
records only what was volunteered. Making claims mandatory (Edit/Write targets must be covered by
the current worktree's claim, with explicit-path claims for ad hoc work) closes that hole but
makes every unplanned one-line edit require a claim first. This is the central
protection-vs-friction tradeoff in the design and it is not currently decided either way.

**D6 — When does a claim actually release? (critical)**
The doc releases at phase checkpoint, but `/plan:delegate` checkpoints create a **local** commit
and explicitly do not integrate to `main`. Releasing there drops protection while the work is
still unmerged, so another branch can edit the same path and meet the conflict later at
integration — the exact outcome this design exists to prevent. Alternative: keep paths reserved
in an `outstanding` state until their commits are ancestors of the trunk, or the user explicitly
abandons them. That is stronger but means claims persist well past the phase that made them, and
a long-lived branch could hold a crate reserved for weeks.

### Auto-recorded, cycle 1 continued (decay/ergonomics lens; R1, R9–R14 independently confirmed)

**R15 — Cache publication needs atomic rename plus a generation stamp. (important; completes R2)**
Publish `reservations.json` by write-to-temp-then-rename, stamped with the journal generation or byte
offset it was built from. The edit hook's clean path then reads a generation-validated cache without
taking the writer lock, so the common case stays fast while correctness still comes from the journal.
Concurrent rebuilds otherwise let an older projection overwrite a newer one.

**R16 — `override` is a fifth verb the command table omits. (mechanical)**
The doc specifies `board | check | claim | release` but the escape hatch it describes has no verb.
Express abandonment as `release --abandon` rather than adding a top-level verb, and specify the
block message's contents: attempted path, overlapping reservation root, claim id, holder worktree
and branch, plan, phase, stated reason, and the exact commands to inspect or override.

**R17 — Tier-2 warnings must be specific, deduplicated, and acknowledgeable. (important)**
Measured: `hana` has **direct path dependencies on 18 workspace crates**, so a Tool Graph claim under
`crates/hana` warns against nearly all Valence library work. A warning that fires on almost every check
is a warning agents learn to skip. Print the exact edge or shortest dependency chain, the holder, plan,
and phase; deduplicate by claim pair and journal generation; distinguish unresolved from acknowledged.

**R18 — Shared root paths need a real record type, not prose. (important; supersedes the bare "announced, not claimed" rule)**
"Announced" currently has no journal operation, no record, no acknowledgement, and no lifecycle, so the
promised "3 branches will touch `Cargo.toml`" report cannot be produced without reparsing mutable plans.
Store root paths as **nonexclusive `shared` reservations** carrying branch, plan, phase, and intended
change. Hooks still allow the edit; the board and checkpoint output retain the relation until resolved.

### Proposed user decisions, cycle 1 continued

**D7 — Does `/plan:delegate` refuse to dispatch a phase with no machine-readable reservation field? (important; sharpens D4)**
Measured against the live plans: Tool Graph phases 60, 69, and 70 have **no `Files` section at all**;
`arrangements.md` Phase 24 uses brace patterns and directory phrases; several phases state outright
that their file set is refreshed during implementation. So there is no deterministic footprint to parse
today. A validated `**Reservations:**` field generated by `/plan:to_phased_plan` and enforced at dispatch
makes claims reliable, at the cost of a new required field in every Work Order and a backfill pass over
both active plans. The softer alternative — derive what can be derived and let the drift tier catch the
rest — keeps plans as they are but leaves the first check of every phase incomplete.

### Auto-recorded, cycle 1 continued (data-model lens; R1–R3, R9–R14 independently confirmed)

**R19 — Journal event schema, concretely. (important; completes R12)**
A versioned tagged union; unknown op or unknown schema version is an error, never a skip. Common
fields on every event: `schema_version`, `event_id`, `at`, `worktree_id`.

| op | required | notes |
|---|---|---|
| `claim` | `reservation_id`, `head_snapshot`, nonempty `scopes`, `source`, `why` | `source` = `WorkOrder { plan, phase }` or `Explicit` |
| `widen` | `reservation_id`, nonempty `added_scopes`, `cause` | `cause` = `Drift { observed_paths }` or `Explicit { why }` |
| `release` | `reservation_id` | optional `why`; `--abandon` variant per R16 |
| `override` | `override_id`, claimant `reservation_id`, nonempty `conflicting_reservation_ids`, nonempty `scopes`, `why` | active only while the claimant **and at least one named blocker** are still active — it can never authorize a later, unrelated claim |

Typed throughout: `EventId`, `ReservationId`, `OverrideId`, `WorktreeId`, `RepoPath`, `Timestamp`,
`PhaseNumber`, `ClaimSource`, `HeadSnapshot`. Only human explanations stay free strings, byte-capped.
This closes concrete hazards: misspelled ops, `"24"` vs `24`, invalid timestamps, short-name
collisions, non-normalized paths. **A phase's several scopes are one reservation**, so conflict
checking and acquisition are atomic across the whole footprint rather than per path.

**R20 — `worktree_id` derivation and `HeadSnapshot`. (important; completes R11)**
`worktree_id` = the worktree's administrative directory from `git rev-parse --git-dir` taken relative
to the common directory, with a dedicated value for the main worktree. Worktree path and
`HeadSnapshot::{Branch { full_ref, oid }, Detached { oid }}` are audit and display data only. Resolve
live branch names and HEAD oids for the board from `git worktree list --porcelain`.

**R21 — The seven-day staleness rule as written is wrong. (mechanical)**
Keying staleness off the branch's base commit marks a brand-new claim stale the moment its branch
starts at an older commit. Derive it as a typed state from the **latest** of: claim time, the most
recent later event on that reservation, and the HEAD commit time.

**R22 — Scope carries an access mode, not just a kind. (important; extends R10)**
`ScopeKind::{File, Tree}` plus `AccessMode::{Exclusive, Announce, ReadOnly}`. Root manifests are
`Announce` (R18); a phase's "verify absent, expect no edit" entries are `ReadOnly`. Brace expressions
are expanded before the block is written. **Compaction may only drop a scope already contained by
another explicit tree scope in the same reservation — it must never invent a common ancestor** (R9).

**R23 — Tier-2 traversal semantics, measured against the hana workspace. (critical)**
Direction, closure depth, dependency kind, and which worktree's manifests to read are all unspecified,
and the live graph shows why that matters: **`hana_valence` dev-depends on `fairy_dust` while
`fairy_dust` depends on `hana_valence`** — recursing through dev-dependencies creates cycles and can
connect most of the 24-member workspace, making nearly every reservation warn about every other. Rules:
- Map a scope to a package by longest package-root ancestor. A scope above all package roots maps to
  every contained member; a repository-only path maps to none. Report an unmapped scope, never skip it.
- Build the graph from **both** the claimant's and the holder's worktrees and union them, so an
  uncommitted manifest edit on either side participates.
- Follow normal and build dependencies transitively. Include each root package's dev-dependencies one
  level, then follow only normal/build edges onward — never dev-dependencies of dev-dependencies.
- Warn in either direction, printing the shortest package path; `claimant → … → holder` means the
  claimant compiles against the holder.
- Deduplicate by reservation pair and dependency path (R17).
- `cargo metadata --format-version 1 --locked --no-deps` suffices for declared workspace path edges:
  **0.02–0.03 s** measured on hana, versus 0.32–0.46 s for fully resolved metadata. Run it once per
  distinct involved worktree per check; no persisted graph cache initially. If a manifest is
  temporarily inconsistent, return `DependencyUnknown` — never a clean tier-2 result.

**R24 — The drift computation, concretely. (critical; completes R8/R14)**
Measured on the hana checkout, the documented command returns **no paths** while git reports three
modified tracked files and two untracked ones — including `docs/worktree-sync.md` itself. Snapshot the
local trunk merge base, then union these NUL-delimited sets:
```
git diff --name-status -z --no-renames "$base"..HEAD     # committed
git diff --cached --name-status -z --no-renames HEAD     # staged
git diff --name-status -z --no-renames                   # unstaged
git ls-files -z --others --exclude-standard              # untracked
```
Parse status records so both sides of a delete/add move stay covered. Bind the check to an explicit
`reservation_id`; implicit selection is valid only when the worktree holds exactly one active
reservation. Widening goes through the same locked overlap transaction as acquisition (R13); a
colliding widen is a typed drift collision requiring an override that names the blocker ids.

**R25 — The board's derived values need a contract. (important)**
Unspecified today: local `main` vs `origin/main`, stored vs current HEAD, how detached or unrelated
heads render, and whether one worktree's several claims repeat the git work. Define trunk as one
snapshotted local `refs/heads/main` oid. Group claims by `worktree_id`, resolve each distinct current
HEAD once, and compute `git rev-list --left-right --count "$trunk_oid...$head_oid"` (first count
behind, second ahead). Return `AheadBehind::{Counts { ahead, behind }, Unrelated, Unavailable}` rather
than inventing zeroes. Report dirty tracked and untracked counts **separately** from ahead/behind, and
flag when current HEAD differs from the claim's snapshot. Measured: `rev-list` ~0.01 s, status
collection ~0.02 s — no board cache is warranted.

## Review findings — cycle 2

### Auto-recorded (cycle 2)

**R26 — R9/R10/R22 verified on real data: concurrency IS preserved. (confirmation, with evidence)**
Cycle 2 normalized the actual Work Orders by hand and produced the reservation sets. After
normalization **Valence 24/25 have no tier-1 overlap with Tool Graph 61–64** — the two plans can run
concurrently, which is the whole point R9 was protecting. Valence 24 reduces to `Cargo.toml`/`Cargo.lock`
announced plus `crates/hana_animation` as a tree plus five `crates/hana_tools` files and two
`crates/hana` files; Tool Graph 62 reduces to six exclusive files across two crates. Recorded because
it converts R9 from an assertion into a measured result.

**R27 — Access modes need a conflict matrix; compaction must be per-mode. (important; completes R22)**
R22 named `Exclusive`/`Announce`/`ReadOnly` without saying which pairs conflict. Foreign-owner matrix:

| holder ↓ / claimant → | Announce | ReadOnly | Exclusive |
|---|---|---|---|
| **Announce** | allow | allow | allow |
| **ReadOnly** | allow | allow | **block** |
| **Exclusive** | allow | **block** | **block** |

**Build a separate antichain per access mode and never compact across modes.** Tool Graph Phase 61 is
the live case: it declares read-only trees but permits one narrowly proven edit inside one of them.
A single-antichain reduction would let the containing `ReadOnly` tree swallow the nested `Exclusive`
file and silently erase the write authorization. Per-mode antichains keep both, and the exclusive
addition still goes through R13's locked overlap check.

**R28 — The lifecycle needs three more events. (critical; completes R19)**
R19's four ops cannot replay the `outstanding` state R14/D6 require: no event records a checkpoint,
so an integrated reservation can never be verified from journal data, and R19 dropped R12's
orphan-retirement operation, so a deleted-worktree reservation stays active forever. Add:
- `checkpoint { reservation_id, result_head, trunk_snapshot }` — transitions `Active → Outstanding`.
- `renew { reservation_id }` — explicit activity for a long phase.
- `release { reservation_id, disposition: Integrated | Abandoned | RetiredOrphan, why }` — terminal
  and auditable; `Integrated` is verified by checking the recorded `result_head` is an ancestor of
  trunk; the other two require confirmation.

Treat **both `Active` and `Outstanding` as live** for overlap checking and for override lifetime.

**R29 — R21's staleness rule contradicts R6. (mechanical; corrects R21)**
R21 said to derive freshness from the latest of claim time, later reservation events, and **HEAD commit
time**. But one worktree may hold several reservations (R6), so any unrelated commit refreshes the
worktree's HEAD and makes *every* abandoned reservation on that worktree look current. **Derive expiry
only from reservation-scoped events** — claim, widen, renew, checkpoint. Drop the HEAD-commit term.

**R30 — Partial-tail recovery must truncate, and cache-ahead is corruption. (critical; completes R2/R3/R15)**
Two gaps in the recorded recovery rules. First, R2 permits *ignoring* an unterminated final record but
never says to remove it — so the next append concatenates onto the partial bytes and converts a
recoverable tail into interior corruption. Second, a byte offset cannot detect an interior edit that
preserves journal length, and cache-ahead was undefined. Full matrix:

| journal ↓ / cache → | current | absent | behind | ahead |
|---|---|---|---|---|
| **present, valid** | use | rebuild | rebuild | **deny** |
| **absent** | deny | coordination inactive | deny | deny |
| **unterminated final record** | truncate tail, rebuild | truncate tail, rebuild | truncate tail, rebuild | deny |
| **corrupt interior** | deny | deny | deny | deny |

Under the mutation lock: replay to the last complete newline; for exactly one unterminated final
record `ftruncate` to that newline and `fsync` **before any append**; treat cache-ahead as `CacheAhead`
corruption requiring `sync repair` rather than trusting either file; stamp the cache with repo
identity, schema version, journal end offset, and a filesystem fingerprint, recomputing a digest on
any fingerprint change; publish through R15's atomic rename.

**R31 — The hook runs tier 1 only. (important; bounds R23/R25 cost)**
Measured: `cargo metadata --locked --no-deps` 0.02–0.03 s per worktree (R23), `rev-list` ~0.01 s and
status collection ~0.02 s (R25). Those belong to `/sync check` and `board`. **The PreToolUse hook must
not call the full three-tier check** — its work is a bounded cache parse plus a journal fingerprint
check, with the lock off the edit path per R1/R15. No performance finding as long as that holds.

### Decision reconciliation (cycle 2)

**D3 — narrowed.** Tool Graph Phase 64's own later correction removes root `Cargo.toml`, root
`Cargo.lock`, and `crates/hana/Cargo.toml` from its file set, so the Phase 64 half of D3's premise is
refuted. D3 still stands on Valence Phases 24 and 32.

**D5 / D7 — evidence, not yet settled.** Cycle 2 parsed all seven upcoming Work Orders against R10/R22:
**six of seven reject.** Valence 24 (brace expressions plus the prose entry "`crates/hana_animation`
tests"), Valence 25 (unexpanded braces), Tool Graph 60 (no `Files` field at all), 61 (authoritative
"Effective Files" conflicts with a later historical `Files` block, plus an alternative location and
unspecified tests), 63 (bare filenames, not repo-relative paths), 64 (braces plus four entries struck
by a later correction). Only Tool Graph 62 parses clean. Phase 60 has no safe fallback: `crates/hana`
as a tree collides with Valence 24's two `crates/hana` files, and an empty fallback protects nothing.

### Auto-recorded (cycle 2, ergonomics/decay lens)

**R32 — Override authorization must ride inside the `claim`/`widen` record. (critical; revises R19)**
R19 requires an override to name the claimant's existing `reservation_id`, but during *initial*
acquisition no such reservation exists yet — R1 rejects the conflicting claim before anything is
appended. Appending a claim and then an override as two events is also crash-unsafe: dying between
them publishes an unauthorized overlap. Embed it instead:
```
claim { reservation_id, scopes, override: { blocker_ids, covered_scopes, why } | none }
```
Surface as `sync claim … --override <ids> --why …`, with the matching `widen` form. **This removes the
standalone `override` event from v1** and supersedes that row of R19's table.

**R33 — A Work Order claim must cover the plan document itself. (important; new interaction bug)**
`/plan:delegate` edits the plan doc during decision resolution, phase review, shrink, and checkpoint,
and may create or edit its `-next.md` sibling. Those are not implementation files, so no `Files`
section names them — meaning under a mandatory-coverage rule the hook would **block `/plan:delegate`
during its own normal checkpoint processing**, and Bash-mediated edits would surface later as
unexplained drift. A Work Order claim therefore auto-includes exact exclusive scopes for its plan
document and derived next-items path, in the same reservation, outstanding with the phase commit.
Session files outside the repository need no reservation.

**R34 — `Announce` and `ReadOnly` do not enter v1. (important; supersedes R18, cancels the access-mode half of R22 and all of R27)**
Measured: only **3 of the 28 remaining phases** name root manifests (Tool Graph 64, Valence 24 and 32).
`Announce` *by construction permits concurrent edits to exactly those files*, so it adds journal
records, board states, acknowledgements, and lifecycle rules while leaving the collision it names
fully possible — it does not achieve the intent. `ReadOnly` reserves no write permission and
duplicates what `Files` already carries. Use **ordinary exact exclusive reservations** for
`Cargo.toml`, `Cargo.lock`, and individual `.claude/config` files for the phase's duration, and keep
verify-only paths in `Files` and out of `Reservations` entirely. R22's `File`/`Tree` distinction and
R9's antichain rule survive unchanged.

*Adjudicating the two cycle-2 lenses:* the correctness lens (R27) built a mode-conflict matrix and
per-mode antichains to handle a `ReadOnly` tree containing a permitted nested edit (Tool Graph 61).
That machinery is only needed because the modes exist. Removing the modes removes the problem rather
than managing it — same correctness, less structure — so R34 wins and **R27 is withdrawn**. If access
modes are ever revived, R27's matrix is the correct specification for them.

**R35 — One shared Work Order parser and validator, not a Reservations-only check. (important; completes D4/D7)**
`/plan:delegate` today accepts the mere presence of `#### Work Order` and copies expected fields
without validating them — Tool Graph 60 lacks the standard Goal/Spec/Files structure and 69/70 lack
`Files`, and none of that is currently caught. A Reservations-only test would leave that failure mode
intact. Define one parser/validator shared by `/plan:to_phased_plan`, `/plan:phase_review`,
pending-decision resolution, and `/plan:delegate`, rejecting missing fields, invalid paths, duplicate
or contained scopes, and malformed Work Orders. Grammar:
```markdown
**Reservations:**
- file: `Cargo.toml`
- tree: `crates/hana/src/transport`
```
Because phase review can later edit remaining Work Orders, one-time generation is insufficient: any
Work Order edit that changes `Files` or adds an implementation path must re-validate `Reservations`
before the writer returns.

**R36 — R7's premise does not hold today. (mechanical; withdraws R7)**
R7 asked for an orchestrator-owns-assignment rule for same-branch fan-out. `/plan:delegate` runs **one
implementation writer at a time**; its concurrent reviewer is read-only. There is no concurrent-writer
case to arbitrate. Revisit only if concurrent writers are introduced.

**R37 — Minimal v1 is 16 of the 25 cycle-1 items. (important)**
Rank A, must exist or the design is unsound: **R1–R5, R8–R13, R15–R16, R19–R20, R24**. Several are one
specification in two entries — R1/R15, R8/R24, R11/R20, R12/R19.
Rank B, deferrable without making v1 wrong: **R6** (stale reservations already fail safe by blocking),
**R14** (trunk-side intersection does not affect acquisition correctness), **R17** and **R23** (ship
with tier 2, the largest independent source of complexity), **R21** (only needed once time-based
staleness reporting exists), **R25** (board metrics do not determine edit permission).
Rank C, dropped: **R7** (R36), **R18** and the access-mode half of **R22** (R34).

**R38 — Day-one adoption order. (important)**
1. Finalize the v1 schema — embedded override authorization (R32), checkpoint state (R28), exclusive
   `File`/`Tree` scopes only (R34), mandatory coverage.
2. Add `Reservations` to the shared Work Order format; update all four writers/readers (R35).
3. Backfill and validate the 28 remaining Work Orders.
4. Build and test journal, mutation lock, replay, generation-stamped cache, worktree identity,
   conflict checks, drift, and repair **in a scratch repository** first.
5. Wire `/plan:delegate` lifecycle across every success, stop, error, and `single` exit.
6. Create the two execution worktrees — `git worktree list` currently shows only
   `/Users/natemccoy/rust/hana`, so no stable worktree identities exist for those plan runs yet.
7. `sync init` once in the shared common directory.
8. Install SessionStart reporting and the PreToolUse hook, then enable mandatory coverage and D7
   refusal **together**.
9. First real claim through `/plan:delegate`. Do not enable the hook before the plans and dispatcher
   are ready.

With this cut the ordinary phased path adds no ritual: dispatch claims automatically, successful edits
produce no hook output, checkpoint preserves the reservation, integration clears it. Manual
interaction is limited to a real collision, an ad hoc edit, or abandonment.

### Decision reconciliation (cycle 2, continued)

**D3 — RESOLVED, dropped from the surfaced set.** `Announce` permits concurrent edits to the very
files it names, so it cannot achieve the intent; the alternative (exclusive for the phase) is the only
option that does. Not a tradeoff. Recorded as R34; risk: three phases briefly hold root manifests
exclusively, which serializes them against each other.

**D1 — sharpened, still open.** The cycle-1 proposal (a `sync exec --paths` declaring wrapper) is
refuted on its own terms: **a wrapper does not constrain what a subprocess writes** — it declares
intent, it does not enforce it. So D1's real choice is narrower than stated: either require repository
source edits through hooked Edit/Write tools and treat unmediated Bash writes as unsupported (with
explicit adapters for trusted mutating commands like formatters), or implement genuine filesystem
write confinement. Command-text parsing is not a third option.

**D7 — backfill cost, measured.** **28 remaining Work Orders**: Tool Graph 19, Valence 9. **25 have a
`Files` block to seed from**; three do not — Tool Graph **60, 69, 70**. All 28 need the field, not just
the three, because none carries `Reservations` today.

### Auto-recorded (cycle 2, risk lens)

**R39 — The worktree administrative path is recyclable; R20's identity is refuted. (critical; corrects R20)**
Git names the administrative directory from the worktree basename and **removes it on
`git worktree prune`**, so a new worktree with the same basename can be handed the same
`worktrees/<name>`. The failure: worktree A holds a live reservation as `worktrees/feature`; A
disappears and is pruned (its reservation correctly survives in the journal); new worktree B takes
`worktrees/feature`, derives A's `worktree_id`, and R11's same-owner exemption hands B a **false
all-clear on A's paths**. Branch metadata cannot catch this because R20 deliberately made branch and
path display-only.

Use an **opaque `WorktreeId`** minted when coordination first sees the worktree and stored inside its
administrative directory, plus a `RepoInstanceId` and the canonical worktree root; the relative
administrative path becomes a locator, not an identity. Before granting same-owner status, validate
repo instance, current administrative directory, its backlink, and the recorded root:

| git state | required result |
|---|---|
| `git worktree move`, or a manual move then `repair` | locked `relocate` event; opaque id preserved |
| common directory moved and repaired | repo/worktree ids preserved; new location resolved |
| administrative directory pruned | old claim orphaned; a recreated directory gets a **new** id |
| recorded path now holds a different repository | old claim orphaned; never inspect it as the claimant |
| administrative directories swapped | identity mismatch / unapproved relocation → deny |
| git linkage unrepaired | R4 internal-error denial |

**R40 — Two coordination runs in one worktree must conflict. (critical; replaces R7's rationale, R36 stands)**
R11 declares the whole worktree one actor and R19 records only `worktree_id`, so **two independent
Claude Code sessions open in the same worktree treat each other as the owner** and can overwrite the
same working-tree state without ever reaching a git merge. R1's mutation lock cannot prevent it:
acquisition is serialized, but both writers are exempted as the same owner. Add a `CoordinationRunId`;
the harness supplies `session_id` to every hook invocation, so it can carry the mapping (teammate
behavior needs testing, with an explicit writer token where session ids are shared). Default to **one
active coordination run per worktree**; a different run in the same worktree conflicts normally.
`/plan:delegate` may grant additional writers only for disjoint scopes or explicit serialization.

*This does not revive R7 as written* — R36 correctly withdrew its fan-out premise, since delegate runs
one implementation writer at a time. The hazard here is two separate sessions, which is unrelated to
fan-out and entirely ordinary (two terminals in one worktree).

**R41 — Release must not be terminal; a trunk rewrite resumes blocking. (critical; extends R28)**
D6's outstanding option releases once the reservation's commit is an ancestor of `main`, but R19's
release event is permanent — so if `main` is later reset or force-moved and the commit is no longer an
ancestor, the reservation stays released while its work is outstanding again. R14 covers only "when
`main` advances"; a rewind or replacement needs a **tree comparison** between the stored and current
trunk oids, since an ahead/behind count identifies no paths. Lifecycle:
```
Active → Outstanding → Integrated
                    ↘ Abandoned
```
`Integrated` **retains its scopes and its integration evidence**, and every check revalidates that
evidence against the current trunk; lost ancestry derives `TrunkRewritten` and resumes blocking until
reconciled. Compare stored and current trunk trees with NUL-delimited `git diff --name-status`
regardless of ancestry direction. Missing objects or unrelated histories return `TrunkUnknown`, never
clear. Squash and cherry-pick integration stay blocked until a user records `integrated-as <trunk-oid>`.

**R42 — The override must escalate to the user, not be executable by the blocked agent. (critical; corrects R16)**
R16 requires the block message to print the exact override command, and nothing requires user
authorization — so an autonomous agent receives the collision, runs the command, and continues. The
collision is journaled but **never surfaced**, which is precisely contrary to the stated intent; the
hook degrades into an audit log. Put `sync override` and `release --abandon` behind a separate
`PreToolUse` rule returning `permissionDecision: "ask"`. The original edit stays denied; after approval
the override command appends the R32 authorization naming exact blocker ids, scopes, reason, and
lifetime. At 2 a.m. with no user response the session **waits, or works only on nonconflicting reserved
paths** — that is the correct outcome for a system whose whole purpose is user-visible collision
resolution.

**R43 — Descriptor-held lock, not a lockfile. (important; completes R1/R30)**
"Advisory lock" does not rule out an existence-based lockfile, which **survives process death** — and
deleting it while the original process is alive creates two writers. Hold `File::lock` on an open
descriptor instead: the kernel releases it when the file closes, including on termination. PID,
process start, and command are diagnostic metadata only. Use bounded `try_lock` retries; a timeout
denies and reports `sync doctor --lock`, and the tool must **never advise deleting the lock file**. A
hook waiting behind a live wedged holder can otherwise exceed its timeout and fail open, so R4's
internal deadline must deny first. Under the acquired lock: truncate an incomplete tail to the last
newline, `sync_all`, then append (R30); ignore incomplete cache temporaries and rebuild from the
repaired journal; fsync the directory after initial journal creation and after the cache rename. Add
termination tests after partial append, journal sync, cache-temporary sync, rename, and directory sync.
A dead holder releases automatically; a live wedged one requires user action and stays fail-closed.

**R44 — R29 confirmed by a third independent lens. (confirmation)**
The risk lens reached R29 separately: R21's HEAD-commit renewal contradicts R6, and a rebase, a ref
update from another worktree, a force-move, or a future-dated commit can keep an abandoned reservation
marked current with no action by its claimant. It still blocks, so this is not a false all-clear — it
is the **old board's decay reappearing as inaccurate freshness**, suppressing the stale condition the
user needs to resolve. Renewal time comes only from reservation journal activity: claim, widen,
explicit renew, checkpoint transition, or approved override. Report HEAD changes independently through
R25, keeping structural state, renewal state, and HEAD relation as three separate typed values.

**R45 — C2-F3's write-authorization matrix is moot under R34, and is its specification if modes return.**
If D5 were implemented as "the target is covered by one of my scopes," a `ReadOnly` scope would
authorize editing the very file it says not to edit, and `Announce` would cover a writable path with
no exclusivity. Since R34 removes both modes from v1, this cannot arise. Recorded for the same reason
R27 was: `Exclusive` satisfies write coverage; `Announce` only for a configured shared-path allowlist
and never returns `Clear`, only `Shared`; `ReadOnly` never satisfies a write and must request a locked
promotion through R13's overlap transaction.

### Auto-recorded (cycle 2, implementation-design lens)

**R46 — R24 used the wrong baseline; partially refuted. (critical; corrects R24)**
R8 and R14 define active-phase changes relative to **the claim's starting HEAD**, but R24 wrote `$base`
as the local trunk merge base — so R24's first command actually computes *outstanding branch
divergence* while its other three compute active-phase staged/unstaged/untracked state. Mixing them
makes earlier checkpointed phases reappear as drift and re-proposes already-released paths for
widening, and makes `Outstanding` indistinguishable from current work. Replace `$base` with
`$phase_start_head` and keep those four results as `ActivePhaseChanges`; compute `OutstandingChanges`
separately from the current trunk merge base. **The three path sets may name the same path and must
stay separately typed — never unioned before policy evaluation.**

Definitive commands:
```bash
# ActivePhaseChanges — committed-since-claim, staged, unstaged, untracked
git diff --name-status -z --no-renames "$phase_start_head"..HEAD
git diff --cached --name-status -z --no-renames HEAD
git diff --name-status -z --no-renames
git ls-files -z --others --exclude-standard

# OutstandingChanges
trunk_oid=$(git rev-parse refs/heads/main)
base_oid=$(git merge-base "$trunk_oid" HEAD)
git diff --name-status -z --no-renames "$base_oid"..HEAD

# Trunk movement since acquisition (exit 1 ⇒ history rewritten, refresh the stored point via `resnapshot`)
git merge-base --is-ancestor "$trunk_at_claim" "$trunk_oid"
git diff --name-status -z --no-renames "$trunk_at_claim".."$trunk_oid"
```

**R47 — Integration is one reachability query. (important; answers the D6 cost question)**
```bash
git merge-base --is-ancestor "$protected_tip" "$trunk_oid"
```
Exit 0 integrated, exit 1 outstanding, anything else unavailable — never clear. **Testing the single
protected tip suffices** because every earlier phase commit is its ancestor: one graph walk, not one
command per branch commit. Measured on hana at **0.01 s across 3,067 reachable commits**. So D6's
outstanding option costs essentially nothing per check.

**R48 — `RepoPath` needs a comparison contract, and hana is case-insensitive. (important; completes R5/R10)**
Verified: the hana repository has **`core.ignoreCase=true`**, while Rust's ordinary component comparison is
case-sensitive — so two claims differing only in component case would receive a false all-clear, and
divergent normalization choices would produce false blocks. Files that do not exist yet also rule out
filesystem canonicalization. Define one `RepoPath::parse` boundary: UTF-8, repo-relative, `/`
separators, no empty / `.` / `..` components, no `.git`, no absolute input. Derive `PathCase` from
repository configuration and compare components under that policy **without canonicalizing through the
filesystem**. Serialize only the normalized spelling.

**R49 — R23's traversal needs explicit visit state and a package-universe rule. (important; completes R23)**
Marking dev-dependencies "root only" does not by itself terminate the live
`hana_valence --dev--> fairy_dust --normal--> hana_valence` cycle. Specify **breadth-first traversal
with each package marked visited before expansion**; only the initial queue item carries
`DevelopmentTraversal::Follow`, every child carries `Skip`, including a back-edge to the root. Define
the package universe as workspace members **plus repository-local dependency paths exposed by
metadata** — `vendor/clay-layout` is one such direct dev dependency of `hana_diegetic`, excluded from
the workspace — treating non-member local packages as leaf nodes unless their manifests are loaded.

Verified on the live graph (24 members). From `hana_valence`: initial expansion `normal: hana_kana`,
`development: fairy_dust, hana_diegetic, hana_lagrange, hana_rubric`; final visited set is
`fairy_dust, hana_clerestory, hana_diegetic, hana_kana, hana_lagrange, hana_rigging, hana_rubric,
hana_valence` — 8 of 24, not the whole workspace. It terminates **only** because the root was marked
visited before expansion. Starting from `fairy_dust`, `hana_valence` is not the root, so its dev
dependencies are skipped.

**R50 — Tier-1 suppression is keyed to the reservation, not the worktree. (important; refines R40)**
Suppress a tier-1 conflict only for **the same reservation**, or for the same `CoordinationRunId` when
the dispatcher recorded file assignment or serialization. Different runs conflict even inside one
worktree (R40). Propagate the active `ReservationId` into hook context alongside the run id. Blocking
naively on same-worktree overlap would make one orchestrated reservation block its own delegates;
ignoring it misses independent-session collisions. Keying on the reservation resolves both.

**R51 — Type skeleton. (important)**
Identifier and encoding choices the doc left open, now fixed: UUID v7 ids, RFC 3339 UTC timestamps,
UTF-8 repository paths, full hex git object ids. Newtypes: `SchemaVersion`, `EventId`,
`ReservationId`, `OverrideId`, `CoordinationRunId`, `CommitOid`, `FullRefName`, `RepoPath`,
`GitAdminPath`, `WorktreePath`, `Timestamp`, `Reason`, `PhaseNumber(NonZeroU32)`. Enums:
`WorktreeId::{Main, Linked}`, `HeadSnapshot::{Branch, Detached}`, `ScopeKind::{File, Tree}`,
`ReservationStage::{Active, Outstanding { protected_tip }}`, `PathCase::{Sensitive, Insensitive}`,
`ClaimSource::{WorkOrder, Explicit}`, `WidenCause::{Drift, Explicit}`,
`ReleaseDisposition::{Integrated, RewrittenIntegration, Abandoned}`,
`AheadBehind::{Counts, Unrelated, Unavailable}`, `DependencyKind`, `DevelopmentTraversal`.
`JournalEvent` carries the common fields with `#[serde(flatten)]` over a
`#[serde(tag = "op")] JournalOperation` union: `Claim`, `Widen`, `Checkpoint`, `Resnapshot`,
`Release`, `Override`, `RetireOrphan`. Two functions: `evaluate_conflicts(candidate,
live_reservations, path_case) -> Vec<ReservationConflict>` and `claim_reservation(repository,
coordination_run_id, claim_request) -> Result<ClaimReceipt, ClaimError>`, the latter taking the
mutation lock internally. `Vec<ReservationConflict>` allocates only during claim/widen transactions,
never on the edit hook's validated-cache path.

**R52 — `Resnapshot` is a fourth new event. (important; extends R28)**
Beyond `checkpoint`, `renew`, and typed `release`, replay also needs an event that **replaces stored
comparison points after a rebase or trunk rewrite** — otherwise the stored `trunk_at_claim` and phase
start can never be refreshed and R41's `TrunkRewritten` state has no way back.
`Resnapshot { reservation_id, snapshot_update: Active { claim_snapshot } | Outstanding {
protected_tip, trunk_oid } }`.

## Decision reconciliation — final

Applying `<DecisionEconomy/>`: an option proven unable to achieve the stated intent is not a plausible
alternative, so it is recorded rather than surfaced. Cost of the work never promotes an item.

**D2 — RECORDED. `/plan:delegate` owns the reservation lifecycle.** Nothing else runs at the right
moments, so no alternative achieves the intent. It claims before the first implementation dispatch,
runs drift detection before checkpoint, emits `checkpoint` (never `release`), releases a cancelled
no-diff phase, and retains and reports reservations after errors, dirty `single` runs, and user stops.
*Risk:* delegate grows lifecycle responsibility, and ad hoc work outside it needs explicit claims.

**D3 — RECORDED, dropped.** `Announce` permits concurrent edits to exactly the files it names, so it
cannot achieve the intent; root manifests take ordinary exclusive reservations for the phase (R34).
*Risk:* the three phases naming root manifests serialize against each other.

**D4 — RECORDED, merged into D7.** The typed `Reservations` field is added, generated and validated by
one shared parser across `/plan:to_phased_plan`, `/plan:phase_review`, pending-decision resolution, and
`/plan:delegate` (R35). Nothing argued against the field itself.

**D5 — RECORDED. Claims are mandatory for every covered write channel.** Voluntary claiming has a race
that no amount of checking closes: A's hook reads generation G and sees no foreign reservation, B
acquires an overlapping reservation and publishes G+1, and A's already-approved edit then executes.
Mandatory acquisition prevents it because A already owns the conflicting reservation before its hook
ever runs. Under the document's own meaning of "safe", voluntary claiming is unsound rather than
merely weaker — so this is not the protection-versus-friction tradeoff cycle 1 recorded.
*Risk:* every unplanned edit needs a claim first; ad hoc explicit-path claims are the release valve,
and the residual gap is D1's.

**D6 — RECORDED. Checkpoint is `Active → Outstanding`, not release.** Releasing at a local checkpoint
lets another worktree edit the same path before integration — the exact collision the design exists to
prevent. This is not hypothetical between the two live plans: **Tool Graph Phase 78 and Valence Phase
27 both name `crates/hana/src/main.rs`** (verified: `tool-graph.md` Phase 78 Files lists
`crates/hana/src/main.rs`; `arrangements.md` Phase 27 Files lists `crates/hana/src/{main,transport}.rs`),
and both also name `crates/hana/src/input/`. R47 measured the integration check at 0.01 s, so the
outstanding option costs nothing per check. Scopes stay exclusive through `Outstanding` until the
protected tip is an ancestor of trunk, or a typed disposition records rewritten integration or
abandonment. *Risk:* a long-lived branch holds paths reserved until it integrates — visible before
either branch edits, which is the point.

**D7 — RECORDED. Dispatch refuses a phase without a valid `Reservations` block.** Follows from D5: a
missing field is already an immediate denial for covered tools, so refusing at dispatch converts a
confusing mid-phase block into a clear error at the right moment. *Risk:* **28 remaining Work Orders
need the field before their plans can run** — Tool Graph 19, Valence 9; 25 seed from an existing
`Files` block, three have none (Tool Graph 60, 69, 70). Enable D5 and D7 together, after the backfill.

**D1 — RESOLVED by the user 2026-08-23.** Hook constrains what it can inspect; a post-write
check observes what actually changed and notifies. See the D1 section below.

### D1 — RESOLVED by the user (2026-08-23): constrain what the hook can see, detect the rest, notify

**The call:** do not over-constrain agents. `PreToolUse` enforces coverage on the write channels it can
actually inspect (Edit/Write). Bash keeps writing freely. A `PostToolUse` check then observes what
actually changed and notifies.

This rejects both extremes cycle 2 offered. It is not "hooked tools only" — Bash stays a first-class
write channel and the auto-mode preference for `sed`/heredocs stands. It is not "accept the gap"
either: cycle 2 framed the Bash residue as drift discovered later at `/sync check`, which means two
branches can both have edited a shared path before anyone is told. Detection moves to **immediately
after the write**, so the window shrinks from a phase to a single tool call.

#### Tier 3 restated

Tier 3 was "drift, self-corrects, computed at check time." It becomes **post-write reconciliation, run
after every Bash call**, with the periodic check retained as a backstop.

Keep a per-worktree working-tree fingerprint in the ledger cache. After each Bash invocation, recompute
it and diff against the stored one — that delta is exactly what this command changed. Classify:

| changed paths | action |
|---|---|
| covered by this worktree's active reservation | silent; update the stored fingerprint |
| covered by no live reservation | auto-widen through R13's locked overlap transaction; report the widen in hook output so the agent knows its footprint grew |
| covered by a **foreign** live reservation | **incursion** — see below |
| would collide on widen (R13) | report the collision; do not widen |

**Incursion is the case that matters.** The write already landed; blocking is no longer available, so
the response is entirely notification and record:
- Append an `incursion { reservation_id, foreign_reservation_ids, paths, at }` journal event, so the
  fact is durable and the other worktree learns about it at its next check rather than at merge.
- Surface it to the user immediately — this is a real collision, which is the one thing the design
  exists to make visible while it is still a sentence.
- Tell the agent in the hook output, so it can stop widening the damage on its own.

Incursions belong on `sync board` as their own state. A branch carrying an unresolved incursion is the
strongest signal the board can show.

#### Cost

This runs after every Bash call, so it must stay cheap. `git status --porcelain` plus
`git ls-files --others --exclude-standard` measured ~0.02 s here (R25), against a cache read — no lock,
no `cargo metadata`, no `rev-list`. Tiers 1 and 2 do not run here. If the fingerprint is unchanged the
hook exits immediately, which is the common case for a Bash call that read rather than wrote.

#### What this does not change

- **D5 stands.** Mandatory coverage still applies to Edit/Write; "do not over-constrain" is about not
  policing Bash, not about relaxing the channel the hook can enforce.
- **R42 stands.** An override still escalates to the user.
- **R46's fuller four-command computation** stays where it was — at `/sync check` and before
  checkpoint. The post-write check is a cheaper fingerprint diff, not a replacement for it.

*Risk carried:* a Bash write to a foreign-reserved path is detected but not prevented, so recovery is
the user's judgment call rather than the hook's refusal. That is the accepted cost of not constraining
Bash, and the incursion record is what keeps it from being silent.

---

## Review findings — cycle 3

Four lenses on the sequencing and self-healing material added in `c37c2924`, which no
prior cycle had seen: sequencing correctness, failure modes, data model and cost,
ergonomics and decay. **Seventeen findings, R53–R69. Zero premise-challenges.** Five
criticals were found independently by three or four of the four agents, which is the
strongest convergence any cycle has produced.

All seventeen are auto-recorded — each had one correct outcome, and the design body
above has been rewritten to carry them. One item does **not** have one correct outcome
and is surfaced as **D8** below.

### Auto-recorded (cycle 3)

**R53 — An overlap answer cannot follow a rejected acquisition. (critical; 3 of 4 agents)**
`sync sequence <first> <then>` names two reservation ids, but under R1 and D5 the second
claim is *rejected* before its id exists, so at the moment of the first collision there
is nothing to name. Appending the claim first opens a crash window containing an
unauthorized overlap; appending the edge first dangles. This is the identical defect
R32 already fixed for `override`, reintroduced. **Fix:** generalize R32 — `claim`/`widen`
carry a `ConflictAuthorization::{Sequence, Defer, Override}` payload; the candidate id is
minted, blockers re-evaluated, cycles validated, and claim plus authorization appended
in one locked transaction. `rescope` is not a journal variant at all; it is a locked
rescope whose ordinary claim events already record it. The standalone `sync override`
verb is removed, restoring R32.

**R54 — Every permissive answer needs user approval, not just `override`. (critical; 3 of 4)**
R42 puts only `override` behind `permissionDecision: "ask"`. `sequence` and the former
`ack` equally suppress a tier-1 block, so as written a blocked agent could pick an order
— or pick none — and unblock itself. The hook would degrade to a journal writer.
**Fix:** all three permissive answers are approval-gated. The prompt names both plans and
phases, the shared paths, the proposed direction, the reason, and the consequence.
`rescope` needs no approval because it leaves nothing overlapping.

**R55 — An answer is bound to scopes and generations, not to a pair of ids. (critical; 3 of 4)**
The text said an answer is recorded "against that specific pair of claims" and also that
"the hook stops blocking those paths" — two different rules. Pair-wide suppression
silently authorizes any *later* overlap the same two reservations create by widening,
turning an R13 widening collision into a false all-clear. **Fix:** every answer stores
both reservation ids, the normalized overlap antichain at the time it was given, and both
reservation generations. Suppression covers only that recorded set; a widen recomputes
and re-blocks anything uncovered; authorization is never transitive to a third
reservation and a new reservation id never inherits one.

**R56 — `ack` was a permanent mute; it becomes `defer`, which holds at integration. (important)**
As written, `ack` had the editing consequences of `override` with no decision, no
enforcement, and no later trigger — the cheapest answer, therefore the one that would be
chosen every time, quietly reproducing the old board's passive-state failure. One agent
proposed deleting it and keeping the paths blocked; that is rejected, because being able
to say "later" without stopping work is the requirement the answer exists to serve.
**Fix:** it unblocks editing and **holds both reservations at integration** until an
order is declared. Same quiet, guarantee intact. Renamed `defer`; `resolve` renamed
`rescope`, which names the act rather than the outcome.

**R57 — `<first-tip>` was undefined, and the commit it names must be retained. (critical; 4 of 4)**
Three readings were available and all three are wrong in different ways: the claim-time
`HeadSnapshot` predates the work (the gate passes before anything lands), the live branch
tip moves with unrelated phases, and either can vanish with the worktree. A stored oid
also does not keep a commit reachable once its branch and admin directory are gone — gc
can take it before lazy reconciliation runs. **Fix:** the subject is the predecessor's
journaled `Outstanding.protected_tip`, never anything else; an `Active` predecessor has
none and holds unconditionally; a retention ref `refs/cargo-berth/reservations/<id>` is
written at checkpoint, updated on resnapshot, and held until every dependent successor is
terminal. A missing object yields `EdgeUnknown` and keeps the successor held.

**R58 — Edge declaration must join the mutation lock. (critical; 2 of 4)**
"Cycles are rejected at declaration" is a read-check-append transaction, but R1's lock
contract names only claim, release, override, and cache rebuild. Two worktrees can each
replay an acyclic graph and append `A → B` and `B → A`; both report success and every
reservation on the cycle is permanently unmergeable. **Fix:** every answer, edge
transition, and reconciliation append runs under R43's descriptor-held lock. Cycle
detection includes edges whose predecessor is currently integrated, because R41 lets
those become pending again.

**R59 — A missing worktree is `Orphaned`, never `Abandoned`. (critical; 4 of 4)**
"A reservation whose worktree is gone gets an appended `abandoned` record" contradicts
R28 (abandonment requires confirmation), R39 (the old reservation stays orphaned), and
this document's own rule that stale claims are flagged and never auto-removed. Worktree
removal does not delete the branch or the commits; absence can equally be a prune not yet
run, a lock, a move, or unrepaired linkage. It would also make a board *read* a mutating
command that terminates ownership. **Fix:** typed liveness
`Live | Unavailable | OrphanCandidate | Orphaned | Unknown`, derived from
`git worktree list --porcelain` plus R39's opaque identity check. Everything but `Live`
retains scopes and edges. A successor is freed automatically only on proven integration
evidence; otherwise the edge waits for user-approved retirement.

**R60 — Edge status is derived, never a terminal record. (critical; 4 of 4)**
Journalling `edge_satisfied` as an appended fact makes it permanent, but R41 already
establishes that integration evidence must be revalidated after every trunk rewrite. A
reset that removes the predecessor from `main` would leave the successor authorized by a
record describing a world that no longer exists. **Fix:** an edge is pending, met, or
cancelled as a pure function of the predecessor's lifecycle and *current* trunk. If a
satisfaction observation is journalled at all it carries its protected tip and trunk oid
and is invalidated whenever trunk changes. Only user-approved abandonment cancels an edge
permanently.

**R61 — Losing the middle node has a defined result. (important)**
`A → B → C` with `A` lost was specified; `B` lost was not, leaving three plausible
implementations (`C` waits forever, `C` is freed, or an `A → C` edge is synthesized).
**Fix:** incident edges are evaluated independently — terminate `A → B`, resolve `B → C`
from `B`'s stored evidence, and never synthesize `A → C`. `C` waits on `A` only if that
edge was declared.

**R62 — Reconciliation cannot depend on someone running `sync board`. (important; 2 of 4)**
Liveness was verified only on board read, in a design that is explicit about having no
mandatory ritual. A dead predecessor could hold its successor indefinitely because nobody
looked. **Fix:** one shared reconciliation routine runs at SessionStart, before every
stateful `/sync` verb, and before every checkpoint and integration. The edit hook keeps
R31's fast path — it reconciles only when the cache says it should block, then retries
the decision once.

**R63 — Orphaned work needs a durable alert, not a message. (important)**
"Surfaces to the user" has no audience, lifetime, or accuracy: detection happens on a
board read, when there may be no session; and worktree removal usually does *not* lose
commits, since the branch ref commonly survives. **Fix:** a persistent
`OrphanedOutstanding` alert, shown at SessionStart, from every `/sync` verb, and whenever
a hook evaluates the orphan or a successor, until the user records recovery, integration,
or approved abandonment. It reports reservation id, protected tip, branch-ref status,
object availability, retention ref, and one of `RecoverableFromBranch`,
`RecoverableFromProtectedTip`, `CommitUnavailable`.

**R64 — Ledger loss fails open for editing and closed for integration. (critical)**
R3 correctly refuses to brick editing when the ledger is gone. Sequencing changes the
stakes on one path: a lost journal erases a *user-approved merge order*, and proceeding
past a decision the user made is not the same risk as proceeding past an inferred claim.
**Fix:** keep R3 for editing; integration fails closed on an absent, corrupt, or
unknown-epoch journal, and reinitializing requires the user to confirm pending orders were
reviewed or reconstructed. The ledger location does not change.

**R65 — The reservation's terminal record is the sole authority for its edges. (important)**
Reconciliation logically emits an abandonment plus one record per outgoing edge, and a
process can die after any complete append. R30 repairs a malformed final record but not a
valid prefix holding half a multi-record transition — which could free a successor before
the decision freeing it is durable. **Fix:** append the reservation's terminal record
first, under the lock, and derive every incident-edge outcome from it at replay. Per-edge
records are audit observations, regenerated idempotently, never required for a correct
projection.

**R66 — The board renders constraints, not a queue. (important; 3 of 4)**
A DAG is a partial order. For `A → B` with independent `C`, both `A,B,C` and `C,A,B`
satisfy it, so a numbered list invents a constraint the user never recorded — and with no
edges at all an empty queue reads as an all-clear, which this document elsewhere calls
worse than a stale row. **Fix:** show the ready set, each held reservation with its named
predecessors and covered paths, unresolved overlaps, and unconstrained live reservations.
Ties within a readiness level are labelled unordered. An empty graph says "no integration
order declared."

**R67 — Sequencing's cost is real and bounded; name it. (important)**
"So it costs nothing new" is wrong — a successor can have several predecessors, and one
worktree can retain several phase reservations, so `V` is not bounded by worktree count
and `E ≤ V(V−1)/2`. A naive per-edge board implementation costs ≈`0.01·E` s and grows
quadratically. **Fix:** rebuild adjacency during replay in `O(J+V+E)`; detect cycles with
a plain DFS in `O(V+E)` — no graph library; group readiness by predecessor for one
`worktree list` plus at most `P` ancestor checks (≈`0.01·P` s); check only a successor's
`d` prerequisites at integration; reject duplicate edges; extend R4's limits to `V` and
`E`. No graph traversal or git subprocess on the hook path.

**R68 — The new operations fold into R51's union rather than extending it. (important; 2 of 4)**
R51's `JournalOperation` contains none of `sequence`, `ack`, `edge_satisfied`,
`edge_dissolved`, and R19 makes an unknown operation an error — so an implementation
following R51 would reject the new records and, under R4, deny editing until repaired.
**Fix:** no new terminal variants. The authorization enum from R53 rides `Claim`/`Widen`;
abandonment reuses R28's typed `ReleaseDisposition`; edge outcomes are derived per R60.
Schema stays version 1 because nothing is built yet. Post-v1 encoding changes increment
`SchemaVersion`, readers reject unsupported versions, and compatible readers deploy before
a writer appends the new version.

**R69 — The successor must actually incorporate the predecessor. (important)**
"You will rebase onto `<first>` … it stops treating its version of the shared file as
final" was aspirational: the only enforcement checked whether the predecessor reached
trunk, never whether the successor contains it, and a message delivered mid-phase gives an
agent nothing to act on. By this document's own standard it must not legislate behavior it
cannot detect. **Fix:** after the predecessor integrates, the successor stays held until
the predecessor's protected tip is an ancestor of the successor's `HEAD`; otherwise it
rebases onto current `main`, emits R52's `Resnapshot`, and reruns `sync check`. The claim
about changing mid-phase behavior is deleted.

**Confirmed, no change:** ordering stays conflict-time state. A Work Order declares its
reservations and never its expected order — the relationship exists only while two
reservations are live, so it is not knowable when the plan is written. The 28-Work-Order
adoption cost is unchanged by sequencing.

### D8 — RESOLVED by the user (2026-08-23): gate the ref, design the valve

**Class:** `design-improvement`. **Severity:** critical. **Found by:** sequencing
correctness and failure-modes lenses, independently. **Status:** `resolved` — enforce.

**Problem.** "The later worktree cannot land first" is currently unenforceable. The
`PreToolUse` hook governs `Edit`/`Write` only, `/sync` had no integration verb, and D1
deliberately leaves `Bash` unconstrained — so `git merge`, `git rebase`, or
`git update-ref` updates `refs/heads/main` without the reachability gate ever running.
The check as written observes state; it does not enforce it. Adding `sync integrate` as
the blessed path (done above) makes the guarantee *available*, not *binding*.

**Why this is not auto-recorded.** The obvious hardening — a `reference-transaction`
hook in the common git directory that rejects any direct `main` update lacking a one-use
permit from `sync integrate` — is exactly the class of hard constraint D1 declined for
editing. Both readings are coherent, and they differ in what they cost when wrong:

- **Enforce.** A ref-transaction hook makes the ordering guarantee real. Cost: every
  `main` update in every worktree goes through a gate, including ones with nothing to do
  with coordination, and a broken or slow hook blocks all integration.
- **Detect and notify**, consistent with D1. `sync integrate` is the supported path; an
  out-of-band `main` update is caught by the next reconciliation and reported as an
  incursion. Cost: the ordering guarantee is advisory, and the notification arrives after
  the merge it was meant to prevent.

Editing and integrating are not obviously the same case — an unwanted edit is a one-line
revert, an out-of-order merge to trunk is the thing this system exists to prevent — which
is why D1's answer does not automatically carry.

#### Resolution

**Enforce.** The `reference-transaction` hook ships, specified under `### The trunk gate`
above. The user's reasoning, recorded: `Bash` was left unconstrained so agents could do
ordinary work without friction, and updating trunk is not ordinary work — it is rare,
deliberate, and the one act that is genuinely painful to undo. D1's answer does not carry
here.

**With a designed release valve**, in the user's words: *"i don't want to be permanently
blocked if we're making some kind of decision to override a policy because we're accepting
the fact that we may have a more complex merge coming."* Two levels —
`sync integrate --force --why` for the deliberate accepted-conflict decision, and
`CARGO_BERTH_BYPASS=1` evaluated before any ledger read so that no failure of this system
can trap anyone, R64's fail-closed integration included. A bypass is journalled and stays
visible on the board; it does not mark the edge satisfied.

**What this adds to v1:** the ref hook, the permit, the two escapes, and the bypassed-edge
board state. R60's derived edge status is unaffected — a permit authorizes one ref update,
it does not change what the edge means.
