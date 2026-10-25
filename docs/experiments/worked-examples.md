# Worked examples

This document applies [the framework](framework.md) to past work. The proposed CLI and hooks did not exist during those studies. Each tooling note states what they would have done.

The past records are evidence, not new measurements. No build, test, model experiment, or machine hold ran for this design.

## 1. Mend and clippy in one compile

### Recorded work

The change put clippy checks inside cargo-mend's compile. It aimed to remove a separate compile during lint. The first cargo-liner timing used three interleaved rounds. Its median fell from 34.9 to 19.9 seconds.

Hana measurements then compared four interleaved rounds. Touching all members gave 50.4 versus 40.7 seconds. Touching one crate gave 24.4 versus 19.7 seconds. The projection reached about one hour per day.

The live review did not confirm that projection. It estimated 5–25 minutes per day, with ranges that included zero. About 40% of live lint calls failed. About 7–9% applied fixes. The benchmark had selected passing calls with no fixes. The machine was also busier after the change.

The user stopped the change on 2026-10-04. The source was reverted. A later install found an experimental binary still installed.

Source for this account and its numbers: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/handoff-mend-clippy-one-compile.md`. Its sections identify the scratch scripts, buildlog query, commits, and live-review limits. Those scratch artifacts were not recovered for this document.

### Required protocol

| Framework item | Required choice |
| --- | --- |
| Question | Does the combined lint path reduce complete live lint time enough to justify its maintenance? |
| A and B | Same Hana commit and lint scope; separate checks versus combined checks |
| Primary measure | Request-to-result wall seconds per assigned lint invocation |
| Correctness | Same required diagnostics, fix behavior, targets, and failure status |
| Controls | Private equivalent targets; paired order; same toolchain and load range |
| A/A | Eight baseline pairs through both preparation paths |
| Sampling | Fixed pairs in input classes selected before execution |
| Live mix | Unchanged inputs, edits, required fixes, and invalid inputs from a fixed store window |
| Costs | Install time, binary size, toolchain updates, example checks, and rollback |
| Daily estimate | Observed frequency times mean saving in each input class |

Define input classes before either arm runs. Do not classify a call as passing from B's result. The combined path can change when a call fails. Removing those failures would remove part of its effect.

The protocol must hold target scope equal. If B also checks examples, that is a second behavior change. Either make A check those examples or register a claim about the combined behavior. Report the added checks and their cost.

The one-crate experiment dropped a contaminated first B run. A registered cache rule would identify that attempt as invalid before comparison. The result would retain the attempt and explain the replacement.

### Decision under this framework

The selected benchmark supports a local execution-time observation. It does not support the projected daily gain. The missing A/A stage and small sample also prevent the framework's controlled acceptance.

The live result would be `not_proven` for the daily gain. The owner could reject adoption because the gain did not justify the added maintenance. The correctness risks of the separate sccache proposal would remain a separate decision.

The framework would probably have found the workload-selection error before installation. That is a counterfactual judgment, not a measured saving. It would also have required an installed-binary check during rollback.

### Tooling action

`register` would require input classes, complete lint scope, and the live rule. `analyze --stage aa` would precede any gain claim. `analyze` would keep failed calls and report missing live support. `decide ... not_proven` would preserve that result.

The start-condition hook would refuse a controlled run above its load limit. The subject hook would show the rejected result before another one-compile proposal. `adopt` would require activation and rollback evidence for both machines.

## 2. Review regime trial

### Recorded work

The trial added a UX reviewer and a code-quality reviewer. The later policy kept the UX reviewer and removed the code-quality reviewer.

The archived ledger has 122 merged-phase rows. The reported comparison kept 9 phases before the trial, 29 during it, and 12 afterward. It excluded other rows after reading the logs. Exclusions included phases without screen changes or usable merge checks.

| Measure | Before | Both extra reviewers | UX reviewer only |
| --- | ---: | ---: | ---: |
| Mean holds per phase | 1.3 | 0.2 | 0.1 |
| Mean merge-check defects per phase | 8.0 | 3.2 | 2.8 |
| Median hours from start to merge | 13.9 | 8.1 | 6.3 |

The report recommends the UX reviewer alone. It also states that the later phases came from more mature plans.

Sources: `~/.claude/docs/as-built/review-regime-trial.md` and `review-regime-trial.jsonl` beside it. The live writer and report are `~/.claude/scripts/production/review_regime.py`. Its code-finding field changed meaning on 2026-10-07. A new comparison must not combine those meanings.

### Required protocol

First separate two questions. One asks whether a UX reviewer helps screen changes. The other asks whether a code-quality reviewer adds value when UX review already exists.

For a supported paired study of the second question, set A to the UX reviewer alone. Set B to UX plus code-quality review. Give both arms the same frozen input and required checks. Register remaining confirmed defects per review task as primary. Define required correctness behavior separately from the scored defect count. Record review wall time, weighted tokens, and incorrect findings as secondary measures.

Select screen-changing tasks before either arm runs. Use one task per independent group and the same scorer rubric in both arms. Keep failed and stopped tasks. A merged-phase-only sample can hide a policy's failed work. The paired task study can establish a review effect. It cannot establish a phase-policy effect from different phases assigned to different arms.

For A/A, have independent reviewers assess the same frozen input under policy A. Measure disagreement and defect-count variation. Use the framework's paired median interval for A/B. Register a minimum positive reduction in confirmed defects left after review. Conceal randomized policy labels from the final scorer until scores are sealed.

The claim that removing a reviewer preserves quality needs a different design. Equal-quality margins and allocation of different phases are unsupported in the first tool version. Do not use the positive-gain rule to accept either claim. A later design must specify its estimator, independent groups, interval, and decision rule. Keep later merge defects and user corrections as observational evidence here.

### Decision under this framework

The historical numbers show an association. They do not separate the policy effect from task mix, plan maturity, or changed exclusions. The causal claim would be `not_proven`.

This does not require reversal of the user's policy decision. The policy can remain an authorized choice with limited evidence. The framework would have reduced the strength of the report's causal language. It would also have required eligibility rules before the trial.

### Tooling action

`register` would refuse the allocated-phase or equal-quality proposal as `unsupported_design`. It could register the narrower paired review-task study above. `run record` would preserve stopped tasks in that study. `analyze` could not accept the historical unpaired comparison. Its report could retain that history as exploratory evidence. `find --subject process/review-regime` would return the narrower result and its limits.

The subject hook would show the policy and its conditions before a review-command edit. The condition hook would detect a changed reviewer model. `run score` would import sealed X/Y scores after each pair. The operator would apply the concealed mapping to the stop rules before the next pair. Missing scores would block that next launch. The hook could not repair the historical allocation.

## 3. Cargo-berth test-time reduction

### Recorded work

The phase targeted 25 tests. A run of 25 filters fell from 8.2 to 3.45 seconds. Individual test times fell by about 30%. Parallel contention made those two observations answer different questions.

Later serial measurements ran at natedev load 59 on 32 cores. Some test timings changed by several times between runs. After merge, CI still had 16 cargo-berth tests at one second or more. The largest time fell from 3.6 to 1.68 seconds.

Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/attempts-phase8-berth-tests-under-a-second.md`. Its CI reference is run `39045724642` at commit `ec3297e9`.

### Required protocol

Choose the claim before the runs. A full-suite improvement uses complete suite wall time as primary. A claim about one slow test uses that test alone. It must also check suite time and coverage.

Measure each target in turn against its prior version. Use the same route and cache preparation. Register a fixed target list. Do not run 25 independent timing jobs at once.

Run A/A first on the intended machine. Use CI as a separate validation population. Do not compare one loaded local sample directly with one CI sample.

Record process starts as a secondary explanation. Fewer starts can reduce contention. The wall measurement must still establish the gain.

The Mac reader-test work supplies a related control example. It alternated each cut with its prior version. It found drift of about 0.2 seconds between batches. It also found that a warmup gave no measured benefit. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/attempts-reader-tests-mac-under-a-second.md`.

### Decision under this framework

The 25-filter result can describe that combined run. It cannot establish the required per-test reduction. A controlled sample at load 59 would be untrusted under the proposed limit.

The evidence would remain `not_proven` for the per-test claim until valid paired samples existed. The later CI result would remain useful evidence of partial improvement. It would not make every target pass.

The framework would have found the wrong measurement unit before the timing gate. It would also have stopped the overloaded serial measurement. It would not have predicted the exact later CI times.

### Tooling action

`register` would require either the suite or one test as the primary unit. `run start` would refuse the controlled load-59 case. `run record` would preserve that refusal. `board` would count zero completed pairs for it.

The condition hook would name the measured load and limit once. The subject hook would show the serial-measurement rule before another test-time phase. It would also link the no-multi-second-tests rule.

A Mac comparison would use `run start --pair` for one serial remote job. The adapter would collect timing and conditions on the Mac. It would copy immutable arm receipts to natedev before `run record --pair` and scoring. A lost connection would require receipt recovery, not another test run. This reuses the explicit remote-status pattern in `~/.claude/scripts/production/mac_run.sh`, `on_mac`, under framework section 12. A lost required condition sample would stop as `not_proven`; the load-breach replacement rule would not apply.

Sources for the gate rules: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/no-multi-second-tests.md`, `reduce-test-load.md`, and `verify-runs-one-at-a-time.md` in the same directory.

## 4. Director model study

### Recorded work

The study compared directors before and after an Opus-to-Sonnet change. It filtered effort and speed. It removed the first request after a model change. It used unchanged directors as a time control.

The pooled continuation median fell from 5.41 to 4.59 seconds. After the control adjustment, the estimated change was -0.11 seconds. Its 95% interval was [-0.53, 0.46] seconds. The report therefore found no measurable adjusted time difference.

Mean API-equivalent cost per request fell from 0.07270 to 0.06005 dollars. The recommendation favored Sonnet on that cost measure. The report also records thin phase evidence.

Sources: `~/.claude/docs/as-built/director-model-study.md`, `director-model-study-results.md`, and `director-model-study-results.json` beside it. The analysis implementation is `~/.claude/scripts/model_study/compare.py`, `phases.py`, and `stats.py`.

### Required protocol

Keep the concurrent control and explicit model filters. Add a pre-registered task-level primary measure. Count all work needed to finish a phase, including compaction and repairs. Include the initial cache cost when it occurs in the proposed policy.

A request-level study can answer a request-latency question. It cannot by itself answer which model completes equivalent phases faster or more cheaply.

Use A/A task pairs to measure task variation. Select one same-input pair per independent group for the interval. The first tool version does not support unpaired phase allocation or combined group estimates. Do not treat hundreds of requests from a few directors as hundreds of independent policy trials.

For historical phase elapsed time, use `phase_started` and `phase_finished` with the same machine and `phase_instance_id`. Verify the run ID. Do not use `history.py phases` field `total_seconds`. It sums stage time and can include overlap or omit gaps. Sources: `~/.claude/scripts/history/history.py`, `_phase_rows`; `~/.claude/scripts/usage/collect.py`, `save_history_event`.

### Decision under this framework

The existing time-control result already rejects a strong reading of the raw speed difference. The framework would retain that useful limit.

The policy-level efficiency claim would remain `not_proven` without matched work and sufficient independent tasks. The lower per-request cost would remain a descriptive finding. It would not automatically establish lower total cost per completed task.

### Tooling action

`register` would distinguish request latency from completed-task efficiency. It would refuse an unpaired policy design. `analyze` would use only independent same-input pairs for acceptance. The historical cost observation would remain exploratory evidence. The subject hook would show the model and effort limits before reusing the result.

## 5. Review-pool reset and fork probes

### Recorded work

The probes compared reset and fresh fork paths. An initial version lost standing instructions. The repaired paths retained the closing rule through six recorded transitions.

In one repaired pair, the first request after reset used 1,434.6 weighted tokens. The first request after fork used 12,981.6. The report states that one pair does not establish future cache behavior.

The probe outputs and scratch rollouts were deleted after use. The report retains the commands and numbers, but not all raw evidence.

Source: `~/.claude/docs/as-built/review-pool-probes.md`. Its weights came from `~/.claude/scripts/usage/weights.py`. The current lifecycle commands are in `~/.claude/scripts/agents/codex_mesh.py`.

### Decision under this framework

Instruction retention is a required correctness check. A cheaper reply that loses instructions fails. The repaired pair supports a candidate for further study. It does not establish a default cost advantage.

The result would be `not_proven` for recurring savings. Its raw-data coverage would be `evidence_unavailable` for the deleted artifacts. The first tool version cannot test that policy claim with these probes. The current policy keeps each unit's reviewer and never resets it. Fresh fork is not its control. Rollback from any later reset trial must restore retained context without reset. Sources: `~/.claude/commands/unit/direct.md`, `LongLivedWorkers`; `~/.claude/scripts/direct/review_pool.py`, `_choose`.

Repeated tasks in one retained thread share context and cache history. They form one dependency group. Different unit-and-date combinations also share units or dates. They do not establish independent pairs. A retained-context comparison needs a later design for related observations. `register` must refuse it as `unsupported_design`. No reset adoption follows from the old fork comparison.

### Tooling action

`run record` would retain the initial failures and repaired observations separately. A changed implementation would require a new protocol revision. `analyze` would refuse a general gain verdict from one pair. `adopt` would require retained evidence and a live check.

The subject hook would show the instruction-loss finding before a reset-policy change. The condition hook would stop a run with the wrong launcher hash or model settings.

## 6. First live protocol: a history-command framework exercise

### Status, purpose, and owner

This is a proposed protocol, not authority to run it. Its name is `history-site-startup-v1`. Its purpose is `framework_exercise`. The user chose it as the first live study on 2026-10-25. It starts after the user's design review is complete.

The experiment-framework unit director owns this exercise. The unit ID is `6d3359af-b72b-4918-9168-c05dc60a142b`. SDLC owns the process design. Registration must retain the request that starts execution and names its operator.

The exercise checks registration, pair launch, receipts, condition rules, scoring, analysis, and result lookup. It uses an existing read-only command. No inspected source establishes a recurring report that uses sealed snapshots. This exercise therefore makes no production claim.

Subject: `process/history-command`, `~/.claude/commands/history.md`, and `~/.claude/scripts/history/history.py`. The command file invokes `python3` without `-S`. The script supports the `phases` query and a state-root override. These are verified command capabilities, not evidence of recurring demand. Sources: `~/.claude/commands/history.md`, Run and Subcommands; `~/.claude/scripts/history/history.py`, `_state_root` and `_cmd_phases`.

The proposed B adds `-S`. This option disables automatic `import site` at initialization. Source: `/run/current-system/sw/bin/python3`, `--help`, read on 2026-10-20. Retain the selected interpreter's help and version at registration. No speed or correctness result exists for B.

The study uses no model calls, reviewer calls, builds, tests, installations, holds, or Mac jobs. Tool implementation is separate work. The study gives evidence about the framework, not expected production savings.

### Question, claim, and commands

Question: can the framework complete its paired measurement and decision procedure for a read-only command with one changed option?

The narrow timing claim is that B reduces median command wall time by more than 0.010 seconds on the fixed input. This threshold exercises the noise and interval rules. It is not a minimum useful production gain. Even seven calls at that threshold save only 0.070 command seconds per week. That amount does not justify a person's work on this change.

A uses the current invocation without `-S`. B adds only that option. Both arms use the same executable, source, input, environment, and query arguments.

```text
A: <python> <history.py> phases --skill plan-delegate --status all --format json
B: <python> -S <history.py> phases --skill plan-delegate --status all --format json
```

At registration, bind `<python>` to the absolute executable selected by the unit's current `python3` command. Bind `<history.py>` to the installed script path. Seal its source and local imports. Record exact argument arrays, source commit, local file hashes, executable hash, Python version, OS, architecture, and adapter version. A changed binding stops the study as `not_proven`.

Set `SKILL_HISTORY_STATE_ROOT` to the same sealed input directory for both arms. Record `USER_ZONE`, locale, Python path settings, and site configuration. Keep credentials out of the record. The environment override and JSON output come from `~/.claude/scripts/history/history.py`, `_state_root`, `_events`, `_cmd_phases`, and `_emit`.

Primary measure: `command_wall_seconds`. Use monotonic elapsed seconds from interpreter launch through command exit and output capture. Include startup and input reads. Exclude input preparation, scheduling, scoring, and receipt publication. Keep those costs separately. Do not call this measure user waiting time. The report's `total` column sums stage durations; it is not this measure or phase elapsed time. Source: `~/.claude/scripts/history/history.py`, `_phase_rows`.

### Input and selection

Before registration, seal one copy of the existing `~/.local/state/plan-delegate/runs/*.jsonl` input set. Record the complete file list, paths, sizes, and modification times before and after copying. Refuse registration if the list or metadata changes. Retain every input hash and the copy time. Use this one fixed input for the whole exercise. Do not select a smaller input after observing timing.

Before each pair, copy that sealed input into the pair's private input directory. Verify its hashes before either arm. Both arms read the same directory. Use normal cache state after this copy, named `after_input_copy`. Do not add warmups or clear shared caches. Seal the collector and checker versions before launch. Refuse registration until the proposed pair adapter and score importer exist.

The first eight selected dates supply A/A pairs. The next twelve supply A/B pairs. A selected date is the first UTC date with an owner-assigned quiet window after the required interval. Require at least 24 hours between selected pair starts. Select at most one pair per natedev UTC date. Neither pair may cross midnight. Record skipped dates and their preflight evidence before any output exists. Replacement pairs also require a new date.

Each arm starts a new interpreter process. The program reads sealed data and retains no interpreter or agent state across pairs. Source for the read path: `~/.claude/scripts/history/history.py`, `_events` and `_cmd_phases`.

The dependency group is the natedev UTC date. The analysis assumes residual timing errors are independent across selected dates after these controls. A date alone does not prove independence. A new shared dependency, persistent competing timing job, or tool change stops the study as `not_proven`. Do not remove selected dates after reading their differences. The exercise cannot establish general workload coverage from one fixed input.

### Order, conditions, and stop rules

For A/A, run A twice per pair. Measure signed first-minus-second differences. For A/B, use this fixed order:

```text
AB BA BA AB AB BA AB BA BA AB BA AB
```

Start one launcher job for the whole pair. Its adapter runs the arms sequentially. It measures the gap from the first exit to the second start. The limit is 60 seconds. No agent dispatch, completion delivery, or receipt import occurs between arms. The adapter checks conditions and direct stop rules before starting the second arm. It records prevention evidence if that arm cannot start. The runner may retain a notice for a later dispatch; pairing does not depend on delivery. Source: `~/.claude/scripts/agents/mesh_runs.py`, `deliver`.

Start at natedev one-minute load at most 8 on 32 logical CPUs. The showrunner assigns the window. Refuse a start during another unit's timing work. Sample load before and after each arm. Sample every five seconds when an arm runs longer than ten seconds. Record unrelated work and source hashes.

Use the framework section 6 failure table without exceptions:

- A refusal before the first arm starts launches nothing and consumes no replacement.
- Complete evidence of unrelated load above 16, second-arm preflight load above 8, externally changed input, or a gap above 60 seconds invalidates the pair.
- Missing required measurements stop as `not_proven`; they cannot be replaced.
- Missing receipts block later work as `execution_unknown`; recover original evidence only.
- A confirmed A or A/A correctness failure gives `not_proven`. A confirmed B failure gives `rejected`.

Allow at most two environmental replacement pairs in all. Preserve every collected arm and any prevented arm. Use the same order on the next eligible date. A source or runtime change is terminal, not replaceable. If missing evidence prevents proof of an environmental cause, do not use the replacement allowance.

The target is eight valid A/A pairs and twelve valid A/B pairs. The deadline is 28 calendar days after registration. Each command has a 30-second timeout. Confirm that the timed-out process and its children ended before any later work. Stop at the deadline. Do not extend the study to obtain a preferred result.

### Completion and score gate

Reserve both arm slots and one launch association atomically. The adapter writes separate immutable arm receipts. It seals the pair result before returning to `mesh_runs`. Each executed arm needs its receipt. Each prevented arm needs its cause and proof that it did not start. The existing runner sends its notice before saving its finished state. The adapter's earlier receipts remove that race. Source: `~/.claude/scripts/agents/mesh_runs.py`, `command_supervise`.

On the notice, import the pair result and arm receipts with `run record --pair`. Lost notices use the same result in an explicit recovery action. Missing or conflicting receipts give `execution_unknown`. Do not launch another job to recover them.

The adapter checks exit, timeout, input hashes, and JSON structure between arms. A direct terminal failure prevents the second arm. For a completed pair, the automated checker seals a separate score file. Require exit zero, valid JSON with nonempty `rows`, and byte-identical stdout and stderr across arms. Retain `_meta` and warning text. No normalization may hide a difference. Record the checker hash and both output hashes.

The operator imports that file with `run score`. This records a score event and a stop decision separately from the receipts. The next pair requires `continue_permitted`. Scores still in delivery leave `awaiting_scores`. Confirmed score loss gives `not_proven`. A B output difference under valid conditions rejects B. An unexplained A/A difference gives `not_proven`. Proven external input drift follows the environmental rule. Seal checks for collected outputs even when the second arm was prevented.

Scorer labels are `not_applicable` because the checker compares exact outputs. Human review studies use the concealed-label procedure in framework sections 3 and 6. This exercise does not validate that separate human procedure.

### Analysis and result

Use the framework's paired median and exact 95% order-statistic interval. Define `d = command_wall_A - command_wall_B`. N is the maximum absolute difference of the eight A/A pairs. Stop as `not_proven` if N exceeds 0.010 seconds or the signed A/A interval excludes zero.

For twelve A/B pairs, the interval uses the third and tenth sorted differences. Set `T = max(0.010, N)` seconds. Report whether the lower bound exceeds T. Require complete receipts, conditions, and score gates for that calculation. Report all pairs and losses. A secondary cost limit rejects B if any valid B command exceeds its paired A time by more than 0.050 seconds.

The exercise succeeds as a procedure check when all registered steps produce complete, traceable records. It need not show a faster B. Report that procedural outcome separately from the effect and interval. An incomplete exercise records which step failed and its evidence.

The record's production-gain decision is `not_proven`, even if the narrow timing threshold passes. A confirmed B correctness or cost failure instead gives `rejected`. Set `live_check` to `not_claimed`, frequency coverage to `not_required`, and adoption plan to `not_permitted`. Do not compute weekly savings. No recurring report or usage count is established by this protocol.

### CLI, hooks, and closure

These proposed commands show one pair and the later analysis. Run every registered pair and score gate before final analysis.

```text
cargo experiment find --subject process/history-command
cargo experiment register <protocol.json>
cargo experiment run start <id> --pair aa-01
cargo experiment run record <id> --pair aa-01 --evidence <pair-result.json>
cargo experiment run score <id> --pair aa-01 --evidence <sealed-scores.json>
cargo experiment analyze <id> --stage aa
cargo experiment run start <id> --pair ab-01
cargo experiment run record <id> --pair ab-01 --evidence <pair-result.json>
cargo experiment run score <id> --pair ab-01 --evidence <sealed-scores.json>
cargo experiment analyze <id>
cargo experiment decide <id> not_proven --basis <decision.json>
cargo experiment find --subject process/history-command
```

The subject hook shows prior results only with the framework's checked freshness state. It labels a stale index as stale. The start hook refuses the wrong host, changed binding, or excessive load. The CLI also checks the prior pair's score gate. The completion hook prints a newly recorded breach once. A valid start produces no routine success line.

After closure, retain the exercise report, input, receipts, analysis, and costs. Publish the decision through the journal-generation contract. The final lookup must return that decision with a matching current generation. It must not present the exercise as an adopted default.

This protocol changes no report command or shared setting. There is no adoption or rollout to reverse. Stop the experiment-owned job on a registered stop condition. Confirm that it ended before releasing its execution slot. Preserve the record. Any later production study needs a real owner, recurring workload, useful-gain threshold, live check, and separate registration. The user can choose that study instead of this exercise.
