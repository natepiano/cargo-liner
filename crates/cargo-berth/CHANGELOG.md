# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A disposable replay checkpoint, `replay-checkpoint.json` in the ledger directory, so a command replays only the journal records appended since it was written. The first command after upgrading replays the whole journal once and writes it; a checkpoint from another build, repository, or journal file, or one whose last record no longer matches the journal, is rebuilt the same way. `init --repair-projection` and `init --reinitialize-after-review` delete it.
- `cargo-berth` compacts `journal.ndjson`: merge extent observations that later observations replace for every reservation they name are removed, automatically once 16 MiB of them accumulate or on `init --compact-journal [--json]`, which reports status `journal_compacted` with a `journal_compaction` payload of the records and bytes removed and remaining. Surviving records are unchanged and in order. The compacted journal replaces the original only when both replay alike; a refused compaction reports `ledger_unreadable` and leaves the journal unchanged, and a failed automatic one leaves `journal-compaction-refused.json`, which holds the next automatic attempt back until 16 MiB more is appended and which `init --repair-projection` deletes. The first command after upgrading a ledger with a large journal compacts it once, in several seconds.
- A present non-trunk integration target's worktree holds a cover reservation against its own target. `target_uncovered` reports a present target with waiting reservations and no registered checkout.

- Report `target_missing` when an unlanded live reservation's recorded branch disappears; board JSON now includes each row's `target` and a sorted top-level `targets` list.
- Coordinate exclusive file and tree reservations across a Git repository's
  worktrees with a journal of every mutation and disposable projections.
- Record directed integration order, deferred overlaps, explicit override
  answers, checkpoints, release evidence, recovery decisions, and incursions.
- Guard trunk updates with an observe-or-enforce `reference-transaction` hook
  and report post-commit drift without rejecting an existing commit.
- Inspect current coordination state through a terminal board or its frozen
  `board --json` contract.
- Recover from projection loss, corrupt journals after confirmed review,
  orphaned worktrees, rewritten integration evidence, and deferred bypass
  audit records.

- Record each reservation's local integration target from `claim --target`, the claimant branch's `branch.<name>.cargoBerthTarget` setting, or the repository trunk; `init` pins older claims and `retarget` replaces a live target.

### Changed

- An `INTEGRATION EVIDENCE LOST` notice tells the reader to inspect `cargo-berth board --reservation <id> --json` for the reservation it names, where it pointed at the whole `cargo-berth board --json`.
- The `reference-transaction` trunk gate in observe-only mode states one line per gated target update, naming the ref and how many entering reservations enforcing mode would refuse it for and pointing at `cargo-berth board`, in place of a full refusal for each held reservation. In enforce mode each refusal counts scopes where it listed every path, and the reconciliation alerts follow the refusals once instead of after each one. `cargo-berth integrate` still lists the paths.
- An overlap refusal is shorter. The edit refusal `hook pre-tool-use` prints names each holder on one line with its reservation id, branch and last activity, adding its shared scopes only when they differ from the requested ones; the envelope payload keeps the other holder facts. The answer menu, which a refused edit and a refused `claim` share, states each answer on one numbered line with its exact command and consequence, and drops the paragraph that repeated which answers add an ordering edge. Every answer, command and rule is unchanged.
- `cargo-berth board --json` no longer repeats the complete board as a pretty-printed block in `presentation` beside `payload`, which carries the same data; that copy made up most of the response. The presentation states only the board's notices.
- `hook session-start` no longer publishes the complete board report, which ran to megabytes of mostly resolved history. After its notices it states one overview block: the entry counts of the non-empty `Ready now`, `Waiting` and `Unresolved overlaps` sections, one line with its instruction for each entry naming a reservation the invoking worktree holds, and the commands that read the rest. A board with no notices and none of those entries states nothing.
- `hook post-tool-use` states each notice line once to each reader, a reader being the payload's `session_id` and `agent_id` (none for the main agent). A line already stated to that reader is dropped, an answer with nothing new is silent, and a line with new facts is stated. `hook session-start` resets the main agent's record and removes records unwritten for 7 days. An exhausted ledger-lock deadline after Bash is now silent instead of asking the reader to retry a command it never ran; the next Bash call's drift covers the change.
- The managed `reference-transaction` hook decides with shell builtins alone: a transaction `cargo-berth` has no use for starts no helper process (a `prepared` run still probes the trunk ref with `git show-ref`), where each run used to start `mktemp`, `cat`, `grep`, `awk` and `rm`. An existing repository needs no `cargo berth init`: the first ref transaction the previous hook sends to `cargo-berth` replaces it with the current one, as the first one after any later hook change will. A hook without the managed marker is never rewritten.
- Reconciliation journals one `holder_merge_extent_observed` record per holder checkout's changed merge extent, listing each reservation it applies to, instead of one `merge_extent_observed` record per reservation. A holder with many outstanding reservations no longer repeats the same extent in each record; journals holding `merge_extent_observed` records still replay unchanged.
- Board `journal_byte_offset` can decrease after a journal compaction; order responses by `generation`. `reservation_revision` no longer counts merge extent observations.
- Hook and command cost no longer grows with journal length. Replay decodes each record once, folding reservation state as it goes and keeping only the records ordering, answers, enrollment, permits, the gate decision, and the bypass audit read; an append advances the held replay over the appended bytes instead of reading the journal again. On a 69.6 MB journal the post-tool-use hook drops from 1.43 s to 0.10 s, the same as on a 58.7 MB journal.
- The reference-transaction gate and `integrate` hold every live integration target to the ordering rules. Run `cargo-berth init` again to install the new hook and publish `gate-targets`.

- Ordering edges between reservations with the same target are judged at that target; edges across targets are judged at the repository trunk. The trunk gate holds an update that brings in a successor still held by a cross-target edge.

- Direct `release` leaves a reservation outstanding when its present target is uncovered; explicit `resolve --integrated-as` remains available. Containment judges each lane at its acting target, so a lane does not collide with that target's cover while work from `main` does.

- `release`, `resolve --integrated-as`, drift attribution, and rebase re-anchoring judge a reservation at its recorded target; malformed `--target` now names why it is not a local branch.

- Reconcile judges each reservation at its recorded target. A lane releases when it reaches its integration branch, and its merge extent holds only its own work.

- A linked worktree's `trunk` key no longer affects judgement; only the main worktree's key defines the repository trunk when its configuration exists.

- Release, lost-evidence, orphan, first-touch, edge-rewrite, and `resolve --integrated-as` text names the reservation's integration target instead of trunk, since that branch judges the work; the `--integrated-as` value is shown as `<TARGET_OID>`.

- `board --json` replaces `recorded_overlap_answers` with `live_overlap_answers` and `released_overlap_answer_count`. The list holds only answers to an overlap with another reservation (`enrollment`, `sequence`, `defer`, `override`, `ordering_created_from_deferral`) recorded by a reservation that is still active or outstanding; a released reservation's answers are only counted. A widen onto paths no other reservation holds, or whose overlaps earlier answers already cover, is scope growth and no longer appears as an answer, so the `widen_without_foreign_overlap` and `existing_answers_cover_every_overlap` board entries are gone. The journal keeps every answer record. `output_contract_version` is now 3.

- A released reservation's `merge_extent` in `board --json` and `board --reservation <id> --json` is `{"status": "released", "at_release": <extent>}`, so the extent last observed before release no longer reads as live protection. The single-reservation text labels it `Merge extent at release (not blocking)`. Active and outstanding rows are unchanged.

### Fixed

- `git pack-refs --prune`, which `git gc --auto` and `git maintenance run --auto` run after ordinary commits, no longer reports that the trunk was deleted with no proven rename. Git reports each loose ref it prunes as a deletion after writing it to `packed-refs`; a trunk that still resolves leaves the managed hook unchanged and prints nothing.
- Hook notices carry only the invoking worktree's alerts. A `PostToolUse` drift states only alerts whose reservation the worktree holds, and `SessionStart` also states alerts no live worktree holds; hand-run verbs and the git gate still report every alert.
- An approved `--before`, `--after`, `--defer`, or `--override` answer keeps
  authorizing edits to the files it names when the holder's merge extent grows
  elsewhere, as when the holder first-touches or dirties another file. The
  answer covers its recorded shared scopes whatever the holder's scope revision
  has become, as enrollment already did; a file newly shared with the holder
  still asks for an answer of its own.
- `resolve <id> --retire-orphan --why <reason>` now clears lost integration
  evidence on a released reservation whose work landed where git cannot match
  it. It appends a `replace_release_disposition` record that replaces the
  `integrated` or `rewritten_integration` disposition with `retired_orphan`, so
  the lost-evidence alert stops on the next reconciliation instead of repeating
  on every hook call. The lost-evidence notice and its board JSON `recovery`
  (new `retirement` member) name this route. An older binary cannot replay that
  record, so every machine sharing the repository must run this version before
  anyone uses it.
- `board --json` lists a live endpoint of an unresolved overlap under `waiting`, which now carries each held reservation's row with a `hold` of `ordering_edge` or `unresolved_overlap`; `unresolved_overlaps` drops a pair once both endpoints are released, and its `consequence` (renamed type `DeferralConsequence`) names which side is still held. `output_contract_version` is now 4.
- An incursion recorded against one reservation now covers every reservation of
  its run in its worktree: post-commit drift no longer charges an answered or
  standing overlap again to each sibling the run claims later, `resolve` accepts
  any of those reservations, and the board's `outstanding_count` counts across them.
- A first-touch `check` no longer widens its selected reservation onto a path a
  sibling reservation of the same run and worktree already declares; it reports
  `already_held`.
- A lane merged into trunk ends its run even when its checkout holds an
  uncommitted edit to a tracked `.claude/config/berth.toml`. Uncommitted
  configuration changes no longer count as lane work in merge-extent or
  enrollment observations; committed changes still do.
- A lane whose history criss-crosses with trunk (each merged the other) gets a
  merge extent of the paths merging it would change or conflict on, instead of
  `merge_extent_unavailable` from `git diff --merge-base` failing with
  "multiple merge bases found". Branch paths now come from `git merge-tree
  --write-tree`, so a modify/delete conflict path is still protected.

- An orphaned or released reservation whose protected tip is not in trunk no
  longer suggests `cargo-berth resolve <id> --integrated-as <trunk tip>`. Both
  notices name that trunk as not containing the work; the orphan notice offers
  `--recovered`, `--retire-orphan`, or `--abandon`, and the lost-evidence notice
  asks for a trunk commit that carries the work. `resolve --integrated-as`
  refuses a trunk commit that carries neither the protected tip nor an
  equivalent of its scoped changes.

- A linked worktree added after `cargo-berth init` reads the main worktree's
  untracked `.claude/config/berth.toml` when it has none of its own, rather than
  reporting `unconfigured` and letting the edit hook allow every write in silence.
- The pre-edit hook honors `CARGO_BERTH_BYPASS=1`: it allows the edit before any
  engine invocation and leaves a pending-bypass marker, so a hung engine no longer
  blocks every write with no escape hatch.
- A post-commit drift check names only the paths the commit introduced or the
  working tree holds open, rather than re-reporting a path left unclaimed at every
  later commit on the branch.
- The recovery command an ambiguous first touch prints now carries
  `CARGO_BERTH_SESSION_ID=<session>` when that variable named the session, so
  running it verbatim resolves the ambiguity instead of publishing no mapping
  and refusing the next edit. A session Claude Code named gets the plain
  command, which its Bash calls already bind through `CLAUDE_CODE_SESSION_ID`.
- An incursion observation pairs each entered path with the holders that block it.
  Carried as two independent sets, an answered path stopped matching its own
  incident as soon as an unrelated path added a holder. The `incursion` record now
  writes `blocked_paths`; records in the two-array layout replay unchanged.
- The batched-attribution benchmark no longer sits in the test suite: it timed two
  hand-copied git command lines against each other with a 25ms margin and never
  ran cargo-berth at all.

### Notes

- The initial release coordinates one repository at a time. It does not select
  integration order, track project phases, or provide an editor write hook.
- The trunk gate ships in observe mode. Rejection is enabled per repository.
