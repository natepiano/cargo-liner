# berth issues — ranked index

A single ordered view over the two backlogs, `berth-fix-next.md` and
`berth-structure-next.md`. Those two docs remain the source: this file holds no
detail of its own, only the ordering and the reason for each position.

Ranked by: does it produce a wrong answer, does it leave the operator stuck,
does it invite a future defect, is it debt. `S#` is the numbered item in
`berth-structure-next.md`; `F` is an item in `berth-fix-next.md`, which numbers
none of its own.

| # | doc | item | why here |
|---|---|---|---|
| 1 | berth-structure-next | **S3** worktree added after `init` reports `unconfigured` | Silent loss of protection. The edit gate answers exit 4 with deliberate silence, so every write is allowed and nothing says so. Worktrees are added constantly. |
| 2 | berth-structure-next | **S7** `check`/`claim` do not refuse a foreign same-worktree reservation | Two sessions in one worktree both believe they hold the same paths — the exact condition berth exists to prevent. |
| 3 | berth-structure-next | **S18** incursion computed against current holders | Fabricates accusations. A commit from 08-28 was reported entering a claim created 09-01. Any new claim over a shared file retroactively accuses every prior reservation. |
| 4 | berth-fix-next | **F** pair entered paths with their holders | The signature permits a union no path exhibits, and it already shipped a defect. Journal migration, so it only gets more expensive. |
| 5 | berth-structure-next | **S11** recovery command cannot resolve the refusal that prints it | The engine names the fix and the fix does not work. Reproduced three times by hand. |
| 6 | berth-structure-next | **S9** drift ambiguity between two reservations in one worktree | Unactionable notice on essentially every tool call, indefinitely, because nothing resolves it. |
| 7 | berth-structure-next | **S10** post-commit names an unmodified file as changed | Feeds #6 — `Cargo.lock` appears in the changed set at every checkpoint without being touched. |
| 8 | berth-structure-next | **S5** edit gate honors no bypass at all | A hung engine blocks every write with no escape, which is precisely when one is needed. The wrapper `exec`s, so nothing survives to time out. |
| 9 | berth-structure-next | **S4** `reference-transaction` bakes an absolute worktree path | Shared hook, per-worktree path. Delete the installing worktree and the hook points at nothing. |
| 10 | berth-structure-next | **S21** wall-clock benchmark false-reds on a busy tree | Sits in the every-phase gate and has fired five times under load. Trains the reader to discount a failing gate — that is the real cost. |
| 11 | berth-structure-next | **S12** `board` prints a pointer to its own JSON | It holds the rendered report and directs the reader to run `--json` for it. |
| 12 | berth-structure-next | **S13** two conditions print the same sentence | Different repairs, identical text; the reader cannot tell which happened. |
| 13 | berth-fix-next | **F** trunk rename leaves `berth.toml` stale | Hook refreshed, configuration not — the next `init` silently restores the deleted branch. Rare trigger. |
| 14 | berth-structure-next | **S6** `--integrated-as` unreachable for an orphan | The disposition intended for rewritten integration cannot be reached by the case that needs it. |
| 15 | berth-structure-next | **S2** re-proposal does not say why it re-gated | Correct behavior, same screen twice, no sign anything was rejected. |
| 16 | berth-structure-next | **S20** four replay helpers accept any operation and silently no-op | The split traded a compile error for a silent no-op. Live trap for the next variant added. |
| 17 | berth-structure-next | **S19** bare map whose missing key means "unproven" | Second home for a lookup the crate already names once; absence carries two meanings. |
| 18 | berth-structure-next | **S15** `GitHookProtocol` cannot say which route answered | Why one of the pair has end-to-end exit-status proof and the other has no command-line test at all. |
| 19 | berth-structure-next | **S22** two `uuid_identifier` macros, identical matchers | A reader cannot tell which definition an invocation selects; adding an eighth identifier misfires with no diagnostic pointing at the shadowing. |
| 20 | berth-structure-next | **S1** contradictory proposal token untested | Verified by hand, covered by nothing. |
| 21 | berth-structure-next | **S14** duplicate-incursion hard stop asserted nowhere | Real refusal path, zero coverage since the front end that reached it was deleted. |
| 22 | berth-structure-next | **S17** reservation-id ordering untested | Two surfaces promise ascending order; the guarantee rests on the collection type alone. |
| 23 | berth-structure-next | **S8** README quotes not bound to real renderings | Recurring drift — three rounds each required a hand sweep. The scenario machinery already exists. |
| 24 | berth-structure-next | **S23** `output.rs`, 4,440 lines | Largest module, four readerships, never split. Provable move — frozen corpora pin the strings. |
| 25 | berth-structure-next | **S24** `reconcile.rs`, 2,429 lines, no unit tests | Four unrelated concerns; a replay defect and an attribution defect fail the same integration test. |
| 26 | berth-fix-next | **F** `maximum_reservations` bound | Real, but unreachable at the shipped default of 128. |
| 27 | berth-fix-next | **F** atomic engine and wrapper publish | The wrappers became pass-throughs, which removed most of the version-skew risk this was written for. |
| 28 | berth-structure-next | **S25** `session/` directory with nothing to hold | Pure form. 424 lines, zero submodules. |

## Where the cut falls

Items 1-8 either lose coordination silently or leave the operator unable to
recover. Items 9-15 are corrections to behavior that is already right. Items
16-23 guard against defects not yet written. Items 24-28 can wait indefinitely.

All four `berth-fix-next` items land at 4, 13, 26 and 27, so that document holds
one pressing defect and three that can sit. Every other item in the top eight
lives in `berth-structure-next`, against what the two names suggest.

`berth-structure-next` item 16 does not appear here. It is closed, and is
retained in that document so the deleted post-tool-use timing bound is not
re-derived.
