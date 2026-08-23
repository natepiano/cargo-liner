# cargo-berth — worktree coordination

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Builds `cargo-berth`, a git-worktree reservation engine, in the `cargo-liner` workspace, and wires it into the `hana` repo's Claude Code environment. This plan lives in `cargo-liner` because phases 1–11 build here; phases 12–17 run in `/Users/natemccoy/rust/hana`.

> **As-built disposition: create**

Design record: `/Users/natemccoy/rust/hana/docs/worktree-sync.md` — 69 review findings (R1–R69) and eight resolved decisions (D1–D8). Work Orders below cite it by section; it is a named file every delegate may read.

## Delegation Context

- **Project:**
  - **Track A (engine, phases 1–11):** `cargo-berth` — new binary crate `crates/cargo-berth` in `/Users/natemccoy/rust/cargo-liner` (workspace members `cargo-mend`, `cargo-port`, `cargo-tile`, `tui_pane`). hana-blind. Publishes to crates.io.
  - **Track B (wiring, phases 12–17):** `/Users/natemccoy/rust/hana` — Claude Code integration. Almost no Rust.
- **Stack:** Rust edition 2024, resolver 3. Workspace-inherited `[workspace.package]` and `[lints] workspace = true`. Deps used, all as `{ workspace = true }`: `clap 4.6.6` (derive), `serde 1` (derive), `serde_json 1`, `toml 1.1.4`, `anyhow 1` (binary), `thiserror 2` (error enums), `cargo_metadata 0.23.1` (tier 2, deferred), `ratatui 0.30.2`, `crossterm 0.29.0`, `tui_pane` (path), `chrono 0.4.45`, `tempfile 3.27.0` (dev). Git access is `std::process::Command`, no git library. File locking is `std::fs::File::lock` — **no new dependency**.
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
    docs/worktree-sync.md                        # the design, single source for both tracks
    docs/hana/tool-graph.md                      # 19 todo phases / 20 Work Orders
    docs/hana_valence/arrangements.md            # 9 todo phases / 9 Work Orders
  ```
- **Key files:**
  - `/Users/natemccoy/rust/cargo-liner/Cargo.toml` — workspace manifest. `members = ["crates/*"]` is a glob, so creating `crates/cargo-berth/` registers the member with no root edit. `[workspace.lints.clippy]` and `[workspace.lints.rust]` live here; members inherit via `[lints] workspace = true`.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-tile/Cargo.toml` — the manifest pattern to copy: inherited `authors`/`edition`/`license`/`repository`, explicit `name` + `version = "0.1.0-dev"`, `categories`, `keywords`, `homepage = ".../tree/main/crates/cargo-berth"`, `readme`, `[lints] workspace = true`, every dep `{ workspace = true }`, `tempfile` under `[dev-dependencies]`.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-tile/src/main.rs` — binary pattern: crate-level `//!` doc (required, `missing_docs` is denied), flat `mod` list, `fn main() -> ExitCode { cli::Cli::parse_arguments().run() }`. No `[[bin]]` section — the binary name is the package name via `src/main.rs`.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-tile/src/cli.rs` — clap `Parser`/`Subcommand` pattern including the `cargo berth <verb>` vs `cargo-berth <verb>` dual spelling: `parse_arguments` swallows the extra word cargo injects.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-port/src/project/git/command.rs` — the git-subprocess pattern to follow: `git_command(repo_root) -> Command` with `--no-optional-locks` and `.current_dir()`, and `git_output_logged(repo_root, op, args)` wrapping it with timing/`tracing::trace!`.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-port/src/project/git/constants.rs` — every git binary name, subcommand, flag, and ref prefix is a named `pub(super) const`. Follow this; never inline a git string literal.
  - `/Users/natemccoy/rust/cargo-liner/crates/cargo-port/src/project/git/worktree_group.rs` — existing worktree-grouping code.
  - `/Users/natemccoy/rust/cargo-liner/crates/tui_pane/src/lib.rs` — board TUI foundation, flat re-exports at the crate root. Entry types: `AppContext` (the trait an app implements), `Framework`, `PaneRegistry`, `Renderable`, `Pane`/`PaneFrame`/`FocusedPane`/`PaneChrome`, `Keymap`/`KeymapBuilder`/`Bindings`/`Action`/`KeyOutcome`, `PaneGridLayout`/`Region`/`Viewport`, `StatusBar`/`StatusLine`, `Theme`/`ThemeRegistry`, `Toasts`, `SettingsStore`.
  - `/Users/natemccoy/rust/cargo-liner/README.md` — member row shape: `- [name](crates/name) — description [![crates.io](https://img.shields.io/crates/v/NAME.svg)](https://crates.io/crates/NAME)`.
  - `/Users/natemccoy/rust/cargo-liner/.claude/config/release.toml` — single-package cadence: `/release <crate> X.Y.Z`, deliberately no `workspace_publish`. A path-only dep needs a `[[publish_path_pins]]` entry.
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
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `phase-end /clippy style-only auto-proceed`
- **Invariants:**
  - **Track-A phases run in `/Users/natemccoy/rust/cargo-liner`. Track-B phases run in `/Users/natemccoy/rust/hana`.** Every phase states its repo in its Goal. A track-A phase that has to explain a Work Order means the boundary is wrong.
  - **Track-B phases compile nothing** and have no `verify.sh` line. They verify by exercising the artifact: run the hook shim against a synthetic JSON payload on stdin and assert the decision, `taplo fmt --check` the TOML, JSON-validate the edited settings file, and confirm every backfilled `**Reservations:**` block parses and its paths exist.
  - Workspace lints are inherited, never restated. Denied: `clippy::{unwrap_used, expect_used, panic, unreachable, allow_attributes_without_reason, self_named_module_files, undocumented_unsafe_blocks}`, groups `all`/`cargo`/`nursery`/`pedantic` at `priority = -1`, `rust::missing_docs`, `rust::unsafe_code`. Every `#[allow]` carries a `reason = "..."`. Use `module/mod.rs` directory form when a module has submodules.
  - Every dependency is `{ workspace = true }`; versions live only in the root `[workspace.dependencies]`.
  - **The append-only journal is truth.** `journal.ndjson` is written `O_APPEND` in sub-`PIPE_BUF` records; nothing rewrites or truncates it. `reservations.json` is a disposable projection — rebuildable by replay, deletable at any moment. No code treats it as authoritative or as the only copy of a fact.
  - **The edit-hook path does no git subprocess work.** It reads only the generation-validated projection, blocks solely on tier-1 foreign-branch overlap, and is silent otherwise. Reconciliation touches git and runs at SessionStart, before stateful verbs, and before checkpoint/integration — plus one retry when the cache already says block.
  - **`cargo-berth` never reads a Work Order or any hana-specific format.** No markdown parsing, no plan-doc awareness, no phase numbering. Its interface is paths and reservation ids.
  - **It publishes to crates.io**: a README for a stranger ships in v1; the crate keeps its own version and `CHANGELOG.md`; no path-only dep without a `[[publish_path_pins]]` entry.
  - Ledger loss fails **open for editing, closed for integration**. Stale/orphaned reservations are flagged, never auto-removed. `Cargo.toml`, `Cargo.lock`, `.claude/config/*` are announced, never claimed. The trunk gate ships observe-only; `HANA_SYNC_BYPASS=1` is evaluated before any ledger read.
  - `cargo-berth` does not coordinate its own construction, and the gate installs in hana last.

## Phases

### Phase 1 — Crate scaffold and the frozen command surface  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `crates/cargo-berth` exists, builds, parses every verb, and returns the documented exit codes and JSON shapes with no logic behind them — freezing the contract track B writes against.

**Spec:**

Create the crate following `crates/cargo-tile` exactly: crate-level `//!` doc, `[lints] workspace = true`, all deps `{ workspace = true }`, `version = "0.1.0-dev"`, `homepage = ".../tree/main/crates/cargo-berth"`. No `[[bin]]` section. `parse_arguments` handles both `cargo berth <verb>` and `cargo-berth <verb>` (cargo injects an extra word) — copy `cargo-tile/src/cli.rs`.

Verbs, from `worktree-sync.md` → `### The skill`:

| Verb | Arguments |
| --- | --- |
| `init` | — |
| `board` | `[--json]` |
| `check` | `<paths>...` |
| `claim` | `<paths>... [--before\|--after\|--defer\|--override <blocker>] [--why <text>]` |
| `release` | `<reservation-id>` |
| `sequence` | `<first> <then> --why <text>` |
| `integrate` | `<reservation-id> [--force --why <text>]` |

Exit codes are the contract the hooks depend on: `0` clear, `1` blocked by overlap, `2` blocked by an unsatisfied ordering edge, `3` needs user authorization, `4` ledger unreadable (fail-open for edit paths, fail-closed for `integrate`), `5` usage error.

Every verb takes `--json`. Define the output envelope once as a serde type: `{ "verb", "status", "exit_code", "reservations": [...], "blocked_by": [...], "message" }`. This envelope is frozen at this phase — track B parses it while the engine is still being built.

Define the id newtypes now, serde-derived, opaque `Display`: `ReservationId`, `WorktreeId`, `CoordinationRunId`, `EdgeId`, `EventId`, `Generation`, `SchemaVersion`. See `worktree-sync.md` R51 for the type skeleton.

Every verb returns `status: "unimplemented"` with exit `0` except usage errors. No ledger, no git.

**Files:**

- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/Cargo.toml` — new manifest, copied shape from `cargo-tile`.
- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/src/main.rs` — `//!` doc, `mod` list, `fn main() -> ExitCode`.
- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/src/cli.rs` — clap `Parser`/`Subcommand`, dual spelling.
- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/src/exit.rs` — the exit-code enum, one variant per code above.
- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/src/output.rs` — the JSON envelope type.
- `/Users/natemccoy/rust/cargo-liner/crates/cargo-berth/src/ids.rs` — the id newtypes.
- `/Users/natemccoy/rust/cargo-liner/docs/berth-plan.md` — append the frozen surface to this Work Order's Spec as the durable contract.

**Constraints from prior phases:** None.

**Acceptance gate:** `verify.sh check cargo-berth`, `test cargo-berth`, `lint cargo-berth` all green. A test asserts every verb parses under both spellings, every documented exit code is reachable, and the JSON envelope round-trips through serde.

### Phase 2 — Journal, projection, and the mutation lock  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: durable append-only storage under `$(git rev-parse --git-common-dir)/hana-sync/`, with replay rebuilding the projection and a lock serializing every writer.

**Spec:**

Ledger location: `$(git rev-parse --git-common-dir)/hana-sync/` — one directory shared by every worktree off the trunk. `journal.ndjson` is truth; `reservations.json` is a generation-stamped disposable cache.

`JournalOperation` is a tagged serde union (R51, R68). Define it now with the variants later phases fill in; **no standalone `edge_satisfied` / `edge_dissolved` / `ack` variants ever** — R60/R68 derive edge state instead. Every record carries `EventId`, `SchemaVersion`, actor, and timestamp. An unknown operation or unsupported schema version is an error, not a skip (R19).

Writes: `O_APPEND`, one record per write, kept under `PIPE_BUF`. Nothing rewrites or truncates. Partial-tail recovery (R30): a malformed **final** record is repaired by truncating it; a malformed record anywhere else is a hard error. Do not apply the cycle-1 partial-tail rule that cycle 2 refuted — see R30.

The mutation lock (R43) is a descriptor-held `std::fs::File::lock` on a lockfile in the ledger dir — **not** existence-based, so the kernel releases it if a process dies. **No new dependency**: `std::fs::File::lock` is std. Every mutation runs: acquire → replay → validate → append → sync → publish projection → release.

Projection: replay produces `reservations.json` with a `Generation` that increments on every publish. A reader that sees a generation newer than the one it read retries once. Deleting `reservations.json` must be recoverable by replay alone, and a test proves it byte-for-byte.

Follow `cargo-port/src/project/git/command.rs` for the one git call needed here (`rev-parse --git-common-dir`) and `constants.rs` for naming it.

**Files:**

- `crates/cargo-berth/src/ledger/mod.rs` — ledger location resolution, directory creation, `init`.
- `crates/cargo-berth/src/ledger/journal.rs` — the operation union, append, replay, partial-tail recovery.
- `crates/cargo-berth/src/ledger/projection.rs` — the cache, generation stamping, publish, read-with-retry.
- `crates/cargo-berth/src/ledger/lock.rs` — the descriptor-held mutation lock and the transaction wrapper.
- `crates/cargo-berth/src/git/mod.rs`, `src/git/command.rs`, `src/git/constants.rs` — the git subprocess helper, patterned on `cargo-port`.
- `crates/cargo-berth/tests/ledger.rs` — integration tests over a `tempfile` scratch repo.

**Constraints from prior phases:** Phase 1 froze the id newtypes in `src/ids.rs` and the exit codes in `src/exit.rs`; exit `4` is ledger-unreadable. `init` exists as a parsed verb returning `unimplemented` — this phase implements it.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: `init` creates the ledger in a scratch repo and is idempotent; replay rebuilds a deleted projection identically; a truncated final record is repaired and a corrupt middle record is a hard error; two concurrent writers serialize (both records present, neither lost); a killed process releases the lock.

### Phase 3 — Claims, scopes, and overlap  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `claim`, `check`, and `release` work, with overlap detected correctly at any path depth.

**Spec:**

`ScopeKind::{File, Tree}` — **exclusive only; `Announce` and `ReadOnly` do not enter v1** (R34, which supersedes R18 and cancels the access-mode half of R22 and all of R27).

Overlap is **path-component ancestry, not string prefix**: `crates/hana_kana` must not match `crates/hana_kana_extra`. Compare normalized components. **This repository has `core.ignoreCase = true` while Rust's component comparison is case-sensitive** (R-cycle-2 measurement) — read `core.ignoreCase` and fold case when it is set, or two claims differing only in case will both be granted.

Claims are validated at acquisition: each path exists, normalizes to repo-relative, and the set reduces to a minimal antichain (drop any scope contained by another in the same claim). Reject a claim naming a path outside the repo.

Acquisition is a single locked transaction (R13): acquire lock → replay → compute overlap against all live foreign reservations → on conflict **reject before appending** (exit `1`, naming holder, branch, plan, phase, reason) → otherwise mint `ReservationId`, append, publish.

The root manifest exception: `Cargo.toml`, `Cargo.lock`, and `.claude/config/*` are **announced, not claimed** — reported as information, never blocking. The list is config-driven (phase 12 supplies hana's).

`release` is non-terminal and revalidated — a reservation released at checkpoint moves to `Outstanding`, not gone. Phase 4 implements that lifecycle; here `release` records the disposition.

`check <paths>` runs tier 1 only and returns the JSON envelope without mutating.

**Files:**

- `crates/cargo-berth/src/scope/mod.rs` — `ScopeKind`, `Scope`, normalization, `core.ignoreCase` handling.
- `crates/cargo-berth/src/scope/antichain.rs` — containment, minimal-antichain reduction.
- `crates/cargo-berth/src/reservation/mod.rs` — the reservation record and its journal operations.
- `crates/cargo-berth/src/verb/{claim,check,release}.rs` — the three verbs.
- `crates/cargo-berth/tests/overlap.rs` — the overlap matrix.

**Constraints from prior phases:** Phase 2 provides the locked transaction wrapper in `src/ledger/lock.rs`, the journal union in `src/ledger/journal.rs`, and the git helper in `src/git/`. Every mutation must run inside that wrapper. Exit `1` is overlap-blocked.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: `crates/hana_kana` does not overlap `crates/hana_kana_extra`; a file claim inside a foreign tree claim blocks; two claims differing only in case both block when `core.ignoreCase` is set; a claim reduces to its antichain; a rejected claim appends **nothing** to the journal; announced paths never block.

### Phase 4 — Reservation lifecycle and git evidence  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: reservations move `Active → Outstanding → Integrated`, and every integration question is answered from retained git evidence rather than a live branch tip.

**Spec:**

`ReservationStage::{Active, Outstanding { protected_tip }, Integrated { trunk_oid }, Orphaned, Abandoned }`. `Abandoned` and orphan retirement **require user confirmation** (R28) — nothing reaches them automatically.

At checkpoint, `release` records `Outstanding { protected_tip }` where `protected_tip` is the branch tip at that moment, and writes a retention ref `refs/hana-sync/reservations/<id>` so the commit survives branch deletion and gc (R57). Resnapshot updates it; it is held until every dependent successor is terminal.

`protected_tip` is **the** subject of every reachability question. Never the claim-time `HeadSnapshot` (it predates the work, so the gate would pass before anything landed) and never the live branch tip (it moves with unrelated phases). An `Active` reservation has no protected tip and therefore blocks unconditionally.

Git computations, verbatim from `worktree-sync.md` R46/R47:

```bash
trunk_oid=$(git rev-parse refs/heads/main)
base_oid=$(git merge-base "$trunk_oid" HEAD)
git diff --name-status -z --no-renames "$base_oid"..HEAD      # OutstandingChanges
git merge-base --is-ancestor "$trunk_at_claim" "$trunk_oid"   # exit 1 => trunk rewritten
git merge-base --is-ancestor "$protected_tip" "$trunk_oid"    # integration
```

`Integrated` evidence is **revalidated on every stateful check, never trusted as terminal** (R41). A trunk rewrite that removes `protected_tip` returns the reservation to blocking and emits `TrunkRewritten`; a missing object is `Unknown` and also blocks. `Resnapshot` (R52) records the replacement tip after a rebase.

**Files:**

- `crates/cargo-berth/src/reservation/lifecycle.rs` — the stage enum, transitions, confirmation gates.
- `crates/cargo-berth/src/reservation/evidence.rs` — protected tip, retention ref, reachability, resnapshot.
- `crates/cargo-berth/src/git/refs.rs` — retention ref create/update/delete.
- `crates/cargo-berth/tests/lifecycle.rs` — scratch-repo lifecycle and rewrite tests.

**Constraints from prior phases:** Phase 3 records a release disposition without a lifecycle; this phase gives it one. Phase 2's transaction wrapper and `src/git/constants.rs` naming convention bind. All reservation records already carry `ReservationId` from phase 1.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: a checkpoint writes a retention ref and the commit survives deleting its branch and running `git gc --prune=now`; an `Active` reservation blocks with no protected tip; `Integrated` reverts to blocking after `git reset --hard` removes the tip from trunk; a rebase + resnapshot updates the tip and the ref.

### Phase 5 — Worktree liveness, reconciliation, and orphan alerts  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: a vanished worktree is classified honestly and never silently frees anything, and the repair runs on the paths that consume the state.

**Spec:** Implements R59, R62, R63 — read those in `worktree-sync.md` → `### Self-healing`.

Typed liveness from `git worktree list --porcelain` plus the opaque `WorktreeId` in the admin dir (R39): `Live | Unavailable | OrphanCandidate | Orphaned | Unknown`. **Everything except `Live` retains the reservation's scopes and edges.** A manually `rm -rf`'d worktree stays registered until `git worktree prune`, a locked worktree is deliberately absent, and a pruned path is recyclable — none of these are abandonment, and abandonment requires user confirmation (R28).

One shared `reconcile()` routine, called at SessionStart, before every stateful verb, and before checkpoint and integration — **not** on board read alone (R62). The edit-hook path never calls it except one retry when the projection already says block.

`OrphanedOutstanding` alert: durable, re-shown until the user records recovery, integration, or approved abandonment. Reports reservation id, protected tip, branch-ref status, object availability, retention ref, and one of `RecoverableFromBranch` / `RecoverableFromProtectedTip` / `CommitUnavailable`. "Commits are lost" must be earned, not assumed.

**Files:**

- `crates/cargo-berth/src/worktree/liveness.rs` — the porcelain parse and the typed classification.
- `crates/cargo-berth/src/worktree/identity.rs` — the opaque `WorktreeId` minted into the admin dir.
- `crates/cargo-berth/src/reconcile.rs` — the shared routine and its call sites.
- `crates/cargo-berth/src/alert.rs` — durable alerts.
- `crates/cargo-berth/tests/liveness.rs`

**Constraints from prior phases:** Phase 4 supplies `protected_tip`, retention refs, and reachability; recoverability verdicts are computed from them. Phase 2's lock wraps every append reconcile makes.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: an `rm -rf`'d-but-unpruned worktree classifies `OrphanCandidate` and keeps blocking; a pruned one classifies `Orphaned` and still keeps its scopes; a locked worktree is `Unavailable`; no path reaches `Abandoned` without explicit confirmation; an `OrphanedOutstanding` alert survives a process restart and reports the right recoverability verdict for a surviving branch, a deleted branch with a retention ref, and neither.

### Phase 6 — Overlap answers  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: a blocked claim can be answered, in the same locked transaction that discovered the block, and the answer authorizes exactly what it was shown.

**Spec:** Implements R53, R54, R55, R56 — `worktree-sync.md` → `## Answering an overlap`.

`ConflictAuthorization::{Sequence { direction }, Defer, Override }` rides the `claim`/`widen` transaction as a payload. **There is no standalone `override` verb and no post-hoc answer** (R53, restoring R32): the candidate `ReservationId` does not exist until acquisition succeeds, so an answer appended separately either dangles or opens a crash window holding an unauthorized overlap. `rescope` is not a variant at all — it is an ordinary re-claim with narrower scopes.

Surface: `claim <paths> --before|--after|--defer|--override <blocker> --why <text>`.

All three permissive answers **require user authorization** (R54) — the CLI returns exit `3` with the material the caller needs to escalate: both plans and phases, the shared paths, the direction, the reason, the consequence. `rescope` needs none.

Each answer stores both `ReservationId`s, the **normalized overlap antichain at the moment it was given**, and both `Generation`s (R55). Suppression covers only that recorded set. A widen recomputes the intersection and re-blocks anything uncovered. An answer is never transitive to a third reservation, and a new `ReservationId` never inherits one.

`Defer` unblocks editing and **holds both reservations at integration** until an order exists (R56). It is not `Override` with a nicer name.

**Files:**

- `crates/cargo-berth/src/answer/mod.rs` — `ConflictAuthorization`, the escalation payload.
- `crates/cargo-berth/src/answer/scope_binding.rs` — antichain + generation binding, re-evaluation on widen.
- `crates/cargo-berth/src/verb/claim.rs` — extend with the four flags.
- `crates/cargo-berth/tests/answers.rs`

**Constraints from prior phases:** Phase 3's acquisition transaction is where the payload lands — extend it, do not add a second path. Phase 3's antichain reduction computes the recorded overlap set. Exit `3` from phase 1 is needs-authorization.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: answering a first collision succeeds in one transaction and a crash between validate and append leaves no reservation and no answer; a widen into a path the answer never covered re-blocks; an answer between A and B does not authorize A against C; `Defer` permits both edits and blocks both integrations; every permissive answer returns exit `3` with the full escalation payload.

### Phase 7 — The edge graph  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: ordering edges form a DAG whose status is derived, never stored, and which cannot be made cyclic by concurrent writers.

**Spec:** Implements R58, R60, R61, R67 — `worktree-sync.md` → `### Sequencing`.

Edge record: `before: ReservationId`, `after: ReservationId`, the validated non-empty overlap scope set, reason, and its journal `EventId`. **No `edge_satisfied` or `edge_dissolved` journal variant** (R60, R68) — status is a pure function of the predecessor's lifecycle and *current* trunk:

- predecessor `Active` → awaiting checkpoint, hold
- predecessor `Outstanding` → hold until `protected_tip` is an ancestor of current trunk
- predecessor `Integrated` → met, **revalidated against current trunk on every check** (R41)
- user-approved abandonment / orphan retirement → cancelled
- successor `Integrated` → fulfilled, drops from the active graph

Cycle rejection runs **inside the mutation lock** (R58): replay → validate both endpoints → DFS → append → sync → publish. Two writers replaying an acyclic graph concurrently would otherwise both append and produce a permanently unmergeable cycle. Include edges whose predecessor is currently integrated — R41 lets those become pending again.

Incident edges are evaluated independently (R61). Losing `B` in `A → B → C` terminates `A → B`, resolves `B → C` from `B`'s evidence, and **never synthesizes `A → C`**.

Cost, per R67: adjacency rebuilt during replay in `O(J+V+E)`; DFS cycle check `O(V+E)` — no graph library; readiness grouped by predecessor so a board read is one `git worktree list` plus at most `P` ancestor checks; integration checks only the successor's `d` prerequisites. Reject duplicate edges; extend R4's limits to `V` and `E`. **No graph traversal or git subprocess on the hook path.**

`sequence <first> <then> --why` changes an answer already given; it never creates the first one (that is phase 6).

**Files:**

- `crates/cargo-berth/src/edge/mod.rs` — the edge record, adjacency, derived status.
- `crates/cargo-berth/src/edge/cycle.rs` — DFS detection under the lock.
- `crates/cargo-berth/src/verb/sequence.rs`
- `crates/cargo-berth/tests/edges.rs`

**Constraints from prior phases:** Phase 6 creates edges as a `ConflictAuthorization::Sequence` payload — this phase reads them, it does not add a second creation path. Phase 4's `protected_tip` and revalidation rule decide every status. Phase 5's liveness feeds cancellation.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: concurrent `A → B` and `B → A` declarations produce exactly one edge; a trunk rewrite that removes the predecessor returns a met edge to pending and re-holds the successor; losing the middle node leaves `C` waiting on nothing and no synthesized edge; duplicate edges are rejected; a bench or counted-call test proves the board's git calls scale with distinct predecessors, not with edges.

### Phase 8 — The trunk gate  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: a `reference-transaction` hook makes the ordering hold real, ships observe-only, and can never trap anyone.

**Spec:** Implements D8 and R64 — `worktree-sync.md` → `### The trunk gate`.

`berth init` installs a `reference-transaction` hook into the **common** git directory, so one hook covers every worktree. It fires on every ref update from any source — terminal, agent, slash command, rebase — and `--no-verify` does not skip it (that flag covers only the commit hooks).

The rule is narrow: deny **only** when a live reservation with an unsatisfied predecessor would newly enter `refs/heads/main`. No edges or nothing pending means silence. A denial names the blocking reservation, its plan and phase, the covered paths, and the exact command to proceed.

**Observe-only is the shipped default.** A config flag flips to enforcing. In observe-only the hook evaluates everything, logs what it would have denied and why, and permits.

Release valve, in order of evaluation:

1. `HANA_SYNC_BYPASS=1` is read **before the ledger, the projection, or anything else**. A corrupt journal, a stuck lock, a hook timeout, or a bug in the gate can never block anyone. Journalled when the journal is writable; reported at next SessionStart when it was not.
2. `integrate --force --why <text>` mints a one-use permit consumed by the next `main` update, journalled with actor, time, reason, and the edges skipped.

A hook timeout denies and names the bypass in its message — safe precisely because the bypass always works.

R64: editing stays fail-open on ledger loss; **integration fails closed** on an absent, corrupt, or unknown-epoch journal, and reinitializing requires confirmation that pending orders were reviewed. A bypassed edge stays on the board marked bypassed with its reason and flags its predecessor as *ordered after work that already landed*; it is never recorded as satisfied.

**Files:**

- `crates/cargo-berth/src/gate/mod.rs` — the hook's decision logic, observe-only, timeout.
- `crates/cargo-berth/src/gate/install.rs` — writing the hook into the common dir, idempotently, without clobbering an existing one.
- `crates/cargo-berth/src/gate/permit.rs` — one-use permits and the bypass env var.
- `crates/cargo-berth/src/verb/integrate.rs`
- `crates/cargo-berth/tests/gate.rs`

**Constraints from prior phases:** Phase 7's derived edge status is the only input to the deny decision. Phase 2's ledger location resolves the common dir. Exit `2` is edge-blocked and exit `4` is ledger-unreadable — `integrate` must return `4` and refuse where an edit path would proceed.

**Acceptance gate:** `verify.sh test cargo-berth` green in a scratch repo with two worktrees, including: observe-only permits and logs; enforcing denies an out-of-order merge and permits an in-order one; `HANA_SYNC_BYPASS=1` succeeds with a deliberately corrupted `journal.ndjson`; a `--force` permit is consumed exactly once; `integrate` fails closed on a missing ledger while `check` still succeeds; installing twice does not duplicate the hook and an existing unrelated hook is preserved.

### Phase 9 — Drift detection  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `berth drift` reports what changed against what was claimed, classifying each result the way D1 requires.

**Spec:** Implements tier 3 and the D1 resolution — `worktree-sync.md` → `## D1 — RESOLVED`.

Fingerprint, verbatim from R46:

```bash
git diff --name-status -z --no-renames "$phase_start_head"..HEAD
git diff --cached --name-status -z --no-renames HEAD
git diff --name-status -z --no-renames
git ls-files -z --others --exclude-standard
```

Classification of the changed set:

| Changed paths | Result |
| --- | --- |
| covered by this worktree's active reservation | silent; update the stored fingerprint |
| covered by no live reservation | auto-widen through the locked overlap transaction; report the widen |
| covered by a **foreign** live reservation | **incursion** — record, surface, tell the caller to stop |
| would collide on widen | report the collision; do not widen |

An incursion appends `incursion { reservation_id, foreign_reservation_ids, paths, at }` and gets its own board state. Budget ~0.02s: `git status --porcelain` plus `git ls-files --others --exclude-standard`, no lock, no `cargo metadata`, no `rev-list`.

A widen re-evaluates every existing answer against the new scopes (phase 6) — an answer never covers a path it was not shown.

**Files:**

- `crates/cargo-berth/src/drift/mod.rs` — fingerprint, classification, incursion records.
- `crates/cargo-berth/src/verb/drift.rs`
- `crates/cargo-berth/tests/drift.rs`

**Constraints from prior phases:** Phase 6's scope binding must be re-run on every widen. Phase 3's antichain reduction normalizes the widened set. Phase 1's envelope carries the classification back to the hook.

**Acceptance gate:** `verify.sh test cargo-berth` green, including one scratch-repo case per table row, an untracked new file counting as changed, and a timed assertion that the fingerprint path makes no `cargo metadata` or `rev-list` call.

### Phase 10 — The board  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `berth board` shows integration constraints truthfully, and `--json` gives the same content to a machine.

**Spec:** Implements R66 — `worktree-sync.md` → `### Self-healing`, final bullet.

**A DAG is a partial order; never render a numbered queue.** Sections: **Ready now** (no unsatisfied predecessors), **Waiting** (each predecessor named, with the covered paths), **Unresolved overlaps**, **Unconstrained live reservations**, and alerts (orphaned-outstanding, bypassed edges with reason and date, stale flags). Ties within a readiness level are labelled unordered. With no edges recorded it says **"no integration order declared"** — never an empty queue, which reads as an all-clear.

Also per-reservation: holder worktree, branch, plan, phase, scopes, stage, and ahead/behind vs `main` computed live and never stored.

TUI built on `tui_pane`: implement its `AppContext` trait, register panes through `PaneRegistry`, use `Keymap`/`KeymapBuilder` for bindings and `StatusBar` for the footer. Follow `cargo-tile` for how a `cargo-*` binary embeds the framework. `--json` emits the phase-1 envelope and must not require a terminal.

Board read calls `reconcile()` first (phase 5).

**Files:**

- `crates/cargo-berth/src/board/mod.rs` — the model: sections, readiness levels, alerts.
- `crates/cargo-berth/src/board/tui.rs` — `AppContext` impl, panes, keymap.
- `crates/cargo-berth/src/verb/board.rs`
- `crates/cargo-berth/tests/board.rs` — model tests, headless.

**Constraints from prior phases:** Phase 7 supplies derived edge status and the readiness grouping — the board must not recompute it. Phase 5's `reconcile()` runs first. Phase 8 supplies bypassed-edge state. Phase 1's envelope is the `--json` shape.

**Acceptance gate:** `verify.sh test cargo-berth` green, including: an empty graph renders "no integration order declared"; independent reservations are shown as an unordered tie, never numbered; a bypassed edge appears with its reason and its predecessor flagged; `--json` runs headless with no TTY and matches the rendered content.

### Phase 11 — README, changelog, and publish readiness  · status: todo

#### Work Order

**Goal:** In `cargo-liner`: `cargo-berth` is documented for a stranger and ready to publish, without publishing.

**Spec:** Implements `worktree-sync.md` → `### The README is a deliverable`.

`crates/cargo-berth/README.md`, written for someone who has never heard of hana, `/plan:delegate`, or Claude Code:

- The six commands in first-use order: `cargo install cargo-berth`; `cargo berth init` (creates the ledger in `.git`, installs the trunk hook, writes a default config); `cargo berth claim <paths>`; `cargo berth board`; `cargo berth integrate`; `cargo berth release`.
- A real collision transcript — actual output — showing which branch holds what and the four answers.
- **The honest limitation, stated plainly and not buried.** The trunk gate is a git hook, so merge ordering is enforced for anybody with no discipline required. Editing is different: blocking the write itself is a Claude Code `PreToolUse` hook and is *not* part of this tool; a general user gets a commit-time drift check instead — automatic, but later than blocking the keystroke. A coordination tool that oversells its enforcement is the failure this design exists to avoid.
- The config file, field by field.
- What it deliberately does not do: choose the merge order, track phases, span repositories.

Add the `cargo-berth` row to `/Users/natemccoy/rust/cargo-liner/README.md` under `## workspace members`, matching the existing row shape. Create `crates/cargo-berth/CHANGELOG.md` in the shape its siblings use. Confirm no path-only dependency was introduced without a `[[publish_path_pins]]` entry in `.claude/config/release.toml`. **Do not run `cargo publish`** — publishing waits until track B proves the loop.

**Files:**

- `crates/cargo-berth/README.md` — new.
- `crates/cargo-berth/CHANGELOG.md` — new.
- `/Users/natemccoy/rust/cargo-liner/README.md` — one member row.
- `/Users/natemccoy/rust/cargo-liner/.claude/config/release.toml` — read only; add a pin entry only if a path dep exists.

**Constraints from prior phases:** Every command and its real output comes from phases 1–10 as built; regenerate the transcript from the actual binary rather than transcribing this plan. The config fields are whatever phase 2's `init` writes.

**Acceptance gate:** `verify.sh check cargo-berth` and `lint cargo-berth` green (`missing_docs` is denied, so the crate-level docs must be complete). `cargo publish --dry-run -p cargo-berth` succeeds. Every command in the README runs as written against a scratch repo, and the collision transcript matches real output byte-for-byte.

### Phase 12 — Config and init in hana  · status: todo

#### Work Order

**Goal:** In `hana`: `.claude/config/berth.toml` states this repo's dialect, and `berth init` has created the ledger with the trunk hook installed observe-only.

**Spec:** Install the binary with `cargo install --path /Users/natemccoy/rust/cargo-liner/crates/cargo-berth`. Nothing is published yet.

`.claude/config/berth.toml`, following the shape of `.claude/config/release.toml` and `mirror.toml` — a header comment explaining the tool and this repo's dialect, then per-repo policy only: `trunk = "main"`; the announce-not-claim list (`Cargo.toml`, `Cargo.lock`, `.claude/config/*`); R4's `V`/`E` limits; `gate_mode = "observe"`.

Run `berth init`, then confirm the ledger exists at `.git/hana-sync/` with `journal.ndjson` and `reservations.json`, and that the `reference-transaction` hook is in the **common** git dir.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/config/berth.toml` — new.

**Constraints from prior phases:** Phase 2 defines the ledger layout and what `init` writes; phase 8 installs the hook and defines `gate_mode`. The field names here must match what phase 2's config reader expects — read `crates/cargo-berth/src/ledger/mod.rs` rather than inventing them.

**Acceptance gate:** No `verify.sh`. `taplo fmt --check .claude/config/berth.toml` passes; `berth board --json` runs and reports an empty ledger with "no integration order declared"; the hook file exists in the common dir; a `git merge` into `main` succeeds and the observe-only log records the evaluation.

### Phase 13 — Hook shims and settings wiring  · status: todo

#### Work Order

**Goal:** In `hana`: an agent's `Edit`/`Write` into a foreign claim is blocked before it lands, and a `Bash` write that slips past is detected right after.

**Spec:** Implements D1 — `worktree-sync.md` → `## D1 — RESOLVED`.

Two shim scripts that shell out to the installed binary and translate its exit codes into Claude Code hook protocol. **`.claude/settings.local.json` has no `hooks` key today — create it**, preserving the existing `permissions` and `outputStyle` entries exactly.

`PreToolUse` on `Edit`/`Write`/`NotebookEdit`: read the tool payload from stdin, extract the target path, call `berth check <path> --json`. Exit `0` → say nothing. Exit `1` (foreign overlap) → exit `2` from the shim to block, with a message naming the holding branch, plan, phase, and reason. Exit `3` → `permissionDecision: "ask"`. Exit `4` (ledger unreadable) → **allow**; fail-open for editing is deliberate. It must make no git call and must be silent on the overwhelming majority of edits.

`PostToolUse` on `Bash`: call `berth drift --json`. Silent and auto-widen results say nothing; an **incursion** surfaces to the user and tells the agent to stop. Budget ~0.02s.

Bash is not constrained, only observed — that is the user's D1 decision, not an oversight. Do not add a `PreToolUse` Bash matcher.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/hooks/berth_pre_edit.sh` — new.
- `/Users/natemccoy/rust/hana/.claude/hooks/berth_post_bash.sh` — new.
- `/Users/natemccoy/rust/hana/.claude/settings.local.json` — add the `hooks` key.

**Constraints from prior phases:** Phase 1 froze the exit codes and the JSON envelope — parse the envelope, never scrape prose. Phase 9 defines the drift classifications the post-hook reacts to. Phase 5's reconcile is *not* called by the pre-hook except the single retry when the projection already says block.

**Acceptance gate:** No `verify.sh`. Each shim runs against synthetic stdin payloads covering every exit code and the decision is asserted; `.claude/settings.local.json` is valid JSON and its prior keys are byte-identical; a real claim in a scratch worktree makes a real `Edit` block; corrupting `journal.ndjson` still permits editing.

### Phase 14 — The /sync skill  · status: todo

#### Work Order

**Goal:** In `hana`: `/sync` gives the board, the checks, and the four answers, and is the only thing that reads a Work Order.

**Spec:** A skill wrapping the binary. Verbs `board`, `check`, `claim`, `release`, `sequence`, `integrate` map to the phase-1 surface.

**The Work-Order-to-paths resolution lives here, not in the tool** — this is the boundary that keeps `cargo-berth` publishable. The skill reads a `**Reservations:**` block out of a plan doc and passes plain paths to `berth claim`. Grammar (`worktree-sync.md:942`): `- file: \`Cargo.toml\`` and `- tree: \`crates/hana/src/transport\``. Paths are **repo-relative**, matching the `**Files:**` blocks already on disk.

On exit `3` the skill escalates to the user with the binary's payload — both plans and phases, the shared paths, the direction, the reason, the consequence — and only then re-invokes `claim` with the chosen flag. **An agent never answers its own block** (R54).

No mandatory emit ritual: state is pulled when wanted, never pushed on a schedule.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/commands/sync.md` — new.

**Constraints from prior phases:** Phase 6 defines the four answers and that they arrive as `claim` flags, not as separate verbs — there is no standalone `override` verb. Phase 1's envelope is the parse target.

**Acceptance gate:** No `verify.sh`. `/sync board` renders; `/sync claim --from-work-order docs/hana/tool-graph.md <phase>` resolves a real Work Order to the right paths and claims them; a forced collision reaches the user rather than being self-answered; every path the skill emits exists in the repo.

### Phase 15 — /plan:delegate integration  · status: todo

#### Work Order

**Goal:** In `hana`: a delegated phase claims its reservations before the first implementation dispatch and releases them at checkpoint, without anyone remembering to.

**Spec:** Claim **before the first implementation dispatch**, recording the phase's starting `HEAD` as the fingerprint baseline. Release at the checkpoint boundary that already exists — which records `Outstanding { protected_tip }`, not disappearance (phase 4).

A phase whose Work Order has no `**Reservations:**` block is a hard stop with an actionable message, not a silent skip — a phase that claims nothing is invisible to every other worktree, which is the exact decay this design exists to prevent.

On a blocked claim, `/plan:delegate` stops before dispatching and surfaces the collision the same way it surfaces a pending decision.

**Files:**

- `~/.claude/commands/plan/delegate.md` — claim/release at the two boundaries. **Edit with the Write/Edit tool, never a shell write** — `~/.claude/commands` is a protected path and shell writes fail with `Operation not permitted`.

**Constraints from prior phases:** Phase 14's skill does the Work-Order resolution — `/plan:delegate` calls it rather than parsing markdown itself. Phase 4's release semantics mean checkpoint does not free the paths.

**Acceptance gate:** No `verify.sh`. A dry run over a real todo phase claims the right paths and releases at checkpoint leaving the reservation `Outstanding`; a phase with no `**Reservations:**` block stops with the actionable message; a forced collision stops before dispatch.

### Phase 16 — Backfill 28 Work Orders  · status: todo

#### Work Order

**Goal:** In `hana`: every live Work Order in both plans declares its reservations, so the two plans can be compared before either runs.

**Spec:** 28 `todo` Work Orders need a `**Reservations:**` block: `docs/hana/tool-graph.md` (19) and `docs/hana_valence/arrangements.md` (9). **`done` phases are not touched.**

25 are generable from the existing `**Files:**` block in the same Work Order: take each backticked path, expand brace notation (`{lib,plugin}.rs`), classify as `file:` or `tree:`, and reduce to a minimal antichain. Paths are **repo-relative** — matching what is on disk, not absolute.

Three have no `**Files:**` block and must be authored by reading the phase: **Tool Graph 60, 69, 70**.

Grammar, `worktree-sync.md:942`:

```markdown
**Reservations:**

- file: `Cargo.toml`
- tree: `crates/hana/src/transport`
```

Do not widen a claim to swallow a whole crate to save effort — rolling `crates/hana_*` up to `crates` eliminates all useful concurrency. Claim at the lowest necessary root.

Once backfilled, run `berth check` over all 28 and **report every collision found** — the known one is Tool Graph 78 and Valence 27, which both name `crates/hana/src/main.rs` and both touch `crates/hana/src/input/`. Record the collisions; do not resolve them here.

**Files:**

- `/Users/natemccoy/rust/hana/docs/hana/tool-graph.md` — 19 todo Work Orders.
- `/Users/natemccoy/rust/hana/docs/hana_valence/arrangements.md` — 9 todo Work Orders.

**Constraints from prior phases:** Phase 14's skill parses this grammar — the blocks must satisfy that parser exactly. Phase 3's antichain reduction defines "minimal". `Cargo.toml`/`Cargo.lock` are announced, not claimed, so a Work Order naming them declares them but they never block.

**Acceptance gate:** No `verify.sh`. All 28 blocks parse; every path exists in the repo; no block rolls up above the lowest necessary root; `done` phases are byte-identical; the collision report names TG78/Valence27 and any others.

### Phase 17 — End-to-end proof, then enforce  · status: todo

#### Work Order

**Goal:** In `hana`: two real worktrees prove the whole loop, and only then does the trunk gate start enforcing.

**Spec:** The end-to-end test is the gate on the gate. Create two real worktrees off `main` and prove, in order:

1. A collision **blocks** — worktree B's `Edit` into a path worktree A claimed is refused by the `PreToolUse` hook, naming A's branch and phase.
2. A `sequence` **holds** — answer the collision `--after`, both worktrees edit freely, and B's merge to `main` is refused while A is unmerged.
3. Landing in order **releases** — A merges, B's hold clears only after B has A's `protected_tip` as an ancestor of its `HEAD` (R69), and a rebase + resnapshot satisfies it.
4. `--force` **lands and is visible** — a forced merge succeeds and the board shows the edge bypassed with its reason and flags the predecessor.
5. `HANA_SYNC_BYPASS=1` **works when broken** — with `journal.ndjson` deliberately corrupted, editing still works and a bypassed merge succeeds.
6. A `Bash` write into a foreign claim surfaces as an **incursion**.

Only after all six pass, flip `gate_mode` from `"observe"` to `"enforce"` in `.claude/config/berth.toml`.

Then clean up: remove the test worktrees, and record in `docs/worktree-sync.md` that the design is built.

**Files:**

- `/Users/natemccoy/rust/hana/.claude/config/berth.toml` — `gate_mode = "enforce"`.
- `/Users/natemccoy/rust/hana/docs/worktree-sync.md` — status line: built.

**Constraints from prior phases:** Every prior phase is exercised here. Phase 8's observe-only default is what makes this safe to run against the real repo — do not flip it early. Phase 12 established the ledger and the hook; this phase changes only the mode.

**Acceptance gate:** No `verify.sh`. All six scenarios pass against two real worktrees with transcripts recorded; `gate_mode = "enforce"`; an out-of-order merge to `main` is then refused in a real terminal, and `HANA_SYNC_BYPASS=1` still lands it.
