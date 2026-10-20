# Worked examples

This document applies [the framework](framework.md) to past work. The proposed CLI and hooks did not exist during those studies. Each tooling note states what they would have done.

The past records are evidence, not new measurements. No build, test, model experiment, or machine hold ran for this design.

## 1. Mend and clippy in one compile

### Recorded work

The change put clippy checks inside cargo-mend's compile. It aimed to remove a separate compile during lint. The first cargo-liner timing used three interleaved rounds. Its median fell from 34.9 to 19.9 seconds.

Hana measurements then compared four interleaved rounds. Touching all members gave 50.4 versus 40.7 seconds. Touching one crate gave 24.4 versus 19.7 seconds. The projection reached about one hour per day.

The live review did not confirm that projection. It estimated 5–25 minutes per day, with ranges that included zero. About 40% of live lint calls failed. About 7–9% applied fixes. The benchmark had selected passing calls with no fixes. The machine was also busier after the change.

The user stopped the change on 2026-10-04. The source was reverted. A later install found an experimental binary still installed.

Source for this account and its numbers: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/handoff-mend-clippy-one-compile.md`. Its sections identify the scratch scripts, buildlog query, commits, and live-review limits. Those scratch artifacts were not recovered for this draft.

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

For the second question, set A to the UX reviewer alone. Set B to UX plus code-quality review. Register remaining confirmed defects per assigned phase as primary. Record phase wall time, weighted tokens, repairs, and user corrections as secondary measures.

Select screen-changing phases before assignment. Require the same merge check and scorer rubric in both arms. Record plan stage and task class. Allocate arms within those classes. Keep phases that fail, stop, or do not merge. A merged-phase-only sample can hide a policy's failed work.

For A/A, have independent reviewers assess the same frozen outputs under the same policy. Measure disagreement and defect-count variation. Hide policy labels from the final scorer. A small number of phases cannot establish equal rare-defect rates.

Register an equal-quality margin if the intended claim is that removing a reviewer preserves quality. Specify the observation period for defects found after merge. Set the allowed delay and cost separately.

### Decision under this framework

The historical numbers show an association. They do not separate the policy effect from task mix, plan maturity, or changed exclusions. The causal claim would be `not_proven`.

This does not require reversal of the user's policy decision. The policy can remain an authorized choice with limited evidence. The framework would have reduced the strength of the report's causal language. It would also have required eligibility rules before the trial.

### Tooling action

`register` would reject an unspecified eligibility rule or primary measure. `run record` would preserve stopped and unmerged phases. `analyze` would report insufficient causal evidence for the historical comparison. `find --subject process/review-regime` would return both the decision and its limits.

The subject hook would show the policy and its conditions before a review-command edit. The condition hook would detect a changed reviewer model or missing required scorer evidence. It could not repair the historical allocation.

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

Use A/A phase or task pairs to measure task variation. Use independent task groups for the interval. Do not treat hundreds of requests from a few directors as hundreds of independent policy trials.

### Decision under this framework

The existing time-control result already rejects a strong reading of the raw speed difference. The framework would retain that useful limit.

The policy-level efficiency claim would remain `not_proven` without matched work and sufficient independent tasks. The lower per-request cost would remain a descriptive finding. It would not automatically establish lower total cost per completed task.

### Tooling action

`register` would distinguish request latency from completed-task efficiency. `analyze` would count independent groups. `decide` would keep the narrower cost observation separate from adoption. The subject hook would show the model and effort limits before reusing the result.

## 5. Review-pool reset and fork probes

### Recorded work

The probes compared reset and fresh fork paths. An initial version lost standing instructions. The repaired paths retained the closing rule through six recorded transitions.

In one repaired pair, the first request after reset used 1,434.6 weighted tokens. The first request after fork used 12,981.6. The report states that one pair does not establish future cache behavior.

The probe outputs and scratch rollouts were deleted after use. The report retains the commands and numbers, but not all raw evidence.

Source: `~/.claude/docs/as-built/review-pool-probes.md`. Its weights came from `~/.claude/scripts/usage/weights.py`. The current lifecycle commands are in `~/.claude/scripts/agents/codex_mesh.py`.

### Decision under this framework

Instruction retention is a required correctness check. A cheaper reply that loses instructions fails. The repaired pair supports a candidate for further study. It does not establish a default cost advantage.

The result would be `not_proven` for recurring savings. Its raw-data coverage would be `evidence_unavailable` for the deleted artifacts. The first live protocol below tests the remaining cost question in real review work.

### Tooling action

`run record` would retain the initial failures and repaired observations separately. A changed implementation would require a new protocol revision. `analyze` would refuse a general gain verdict from one pair. `adopt` would require retained evidence and a live check.

The subject hook would show the instruction-loss finding before a reset-policy change. The condition hook would stop a run with the wrong launcher hash or model settings.

## 6. First live protocol: review context after reset

### Status and scope

This is a complete proposed protocol. It is not registered and has not run. The user can select it after approving the design. Registration must copy these rules into the proposed record format. It must bind the runtime identities by the rules below before any measured request.

Protocol name: `review-context-reset-v1`.

Subject: `process/review-pool` and `~/.claude/scripts/agents/codex_mesh.py`.

Owner: the unit that receives the approved experiment Work Order. Its showrunner owns any later shared-default decision. The study changes no production review policy while it runs.

The experiment uses live read-only review tasks. Both answers go to the normal director after blinded scoring. The director keeps responsibility for the actual phase decision. The experiment does not replace a required reviewer or add a code change.

### Question, claim, and measure

Question: does an empty retained review thread reduce weighted usage compared with a fresh fork for equivalent review work?

A: fork a new empty thread from the sealed reviewer template for each task.

B: reset one retained review thread to zero turns before each task. Keep the same standing instructions. Reuse that thread for the next task.

Claim: B reduces median weighted tokens per assigned task by more than 2,000 tokens. B must retain the standing instructions and satisfy every required review check.

Primary measure: weighted tokens from task preparation through the final review reply. Include every model request in that interval. Allocate B's one initial warmup equally across the twelve B tasks. Allocate any A-only setup model usage equally across the twelve A tasks. Prime and fork calls that use no model request contribute zero model tokens, but retain their wall time.

Weights are fixed at uncached input 1, cached input 0.05, and output 5. Thinking, if included in output, is not added again. Report raw token classes beside the weighted value. This is a weighted-usage study, not a dollar or subscription-saving study.

Sources for those historical model weights and request parsing: `~/.claude/scripts/usage/weights.py`, `~/.claude/scripts/usage/rollouts.py`, `~/.claude/docs/as-built/review-pool-probes.md`.

### Frozen implementation and runtime binding

The design read `~/.claude` at commit `fafa6eb22064c0fa8e429f4fae46dd69d8baac7c` on 2026-10-20. File hashes are authoritative if that tree has local edits.

| File under `~/.claude/` | SHA-256 read for this design |
| --- | --- |
| `scripts/agents/codex_mesh.py` | `8e49239304d216193b4eecd3c4dce9664256de8bc6ebc342c93ac45a379b33f2` |
| `scripts/usage/weights.py` | `c4257720ba2cbe0e57325d4cc2a3ee3e8c14e6526c30cf0d44bde25078d1f743` |
| `scripts/usage/rollouts.py` | `8c15c7b6a089f3b4fd87ad36a6310a5ee4f0e4e94080e210469b9352f93e4bd7` |

At registration, verify these hashes. If one differs, stop before any run. Review the new version and register a new protocol revision. Do not execute an old tool against new shared state merely to reproduce a hash.

Bind the installed Codex version, app-server configuration hash, and approved workers account label at registration. Use that same account for both arms. Pin `gpt-6.1-sol`, effort `low`, and service tier `default`. If the server refuses that tier or reports different settings, do not substitute a value. Close as `not_proven` and revise the protocol.

The experiment has one private app-server and one template. Its scratch directory is under the experiment's evidence directory. Normal agents continue to use their existing servers. The registered binding includes the exact directory and launcher identity.

No build hold is needed. No compilation, test, install, or Mac job is part of this protocol. All model work runs from natedev with read-only tools.

### Task eligibility and input selection

Use the next eligible first review from each distinct unit-and-day group. The owning showrunner supplies tasks in ready-time order. Break equal times by unit ID and phase ID. Select at most one task per group. Do not select a task because it is known to favor either arm.

An eligible task must meet all these conditions:

- It is an actual read-only review needed by an existing phase.
- It has one immutable patch or document revision and a fixed source snapshot.
- Its complete review input has 2,000–12,000 UTF-8 words.
- It needs no build, network read, user answer, or file mutation.
- The director can state at least three required checks before either answer exists.
- Its existing review time limit is at least 300 seconds.

Exclude later repair reviews of the same phase. Exclude tasks whose required check depends on hidden external evidence. Record each exclusion and its reason before launch.

The first eight eligible groups supply A/A pairs. The next twelve supply A/B pairs. Require at least three units and three dates in A/B. Keep unit and date labels in the report. Any unresolved dependence between those twelve groups makes the final result `not_proven`. Do not change the grouping after seeing the effect.

For each selected task, seal `input.md`, source files, and `checks.json` before either arm runs. The checklist names required issues or behaviors, severity, and source evidence. The director signs its hash in the task manifest. The scorer cannot add a required check after seeing which arm missed it.

Task source commits vary because these are live tasks. Each pair uses identical source bytes. The manifest records the full source commit and patch hash. A source change between paired runs invalidates the pair.

### Standing instructions and task prompt

Use this exact standing text in both paths:

```text
Review the sealed input for this task.
Read only files in the sealed source directory.
Do not run builds, tests, network commands, or file writes.
Report each confirmed defect with its source path and reason.
Do not report a defect without evidence in the input.
State which required checks pass and which fail.
End the final reply with exactly REVIEW_COMPLETE.
```

The task prompt is the following text plus the sealed input and required-check list:

```text
Review this task under the standing instructions.
Use the attached input and required checks.
Return the review in one final reply when the work is complete.
```

The standing marker does not appear in the task prompt. Its presence tests whether the standing instructions remain available. A marker alone does not prove full instruction retention. Verify the other instructions and required checks as well.

### Preparation and commands

The command forms below come from `~/.claude/scripts/agents/codex_mesh.py`. Values in angle brackets come from the sealed record. They are not open choices during a run.

Create the template once:

```text
~/.claude/scripts/lib/py ~/.claude/scripts/agents/codex_mesh.py prime --session-dir <dir> --name template --instructions-file <standing-file> --cwd <sealed-root> --model gpt-6.1-sol --service-tier default
```

For each A task, create a fresh thread:

```text
~/.claude/scripts/lib/py ~/.claude/scripts/agents/codex_mesh.py fork --session-dir <dir> --from template --name <a-task-name>
```

Before the A/B stage, create B's retained thread once by the same fork command. Give it one warmup task: `Reply in one line: ready.` Record that reply and all its usage. Do not repeat the warmup.

Before every B task, remove its turns:

```text
~/.claude/scripts/lib/py ~/.claude/scripts/agents/codex_mesh.py reset --session-dir <dir> --to retained
```

Use the same follow command for every measured review:

```text
~/.claude/scripts/lib/py ~/.claude/scripts/agents/codex_mesh.py follow --session-dir <dir> --to <thread-name> --role review --model gpt-6.1-sol --effort low --service-tier default --cwd <sealed-task-root> --read-only --message-file <prompt-file> --summary-file <reply-file> --log-file <turn-log> --timeout 300
```

Use separate reply and log paths for every attempt. Keep the reset response and the active rollout path. The earlier probe found that reset can create a new rollout for the same thread. Source: `~/.claude/docs/as-built/review-pool-probes.md`.

Collect requests by their recorded request keys and attempt boundaries. Do not subtract cumulative thread totals across reset. Keep each request once, even if a resumed rollout repeats it. Record missing usage as `collection_failed`; never substitute zero. Sources: `~/.claude/docs/as-built/usage-ledger.md`, `~/.claude/scripts/usage/rollouts.py`.

The completion adapter must verify zero retained turns before B's task begins. It must read the serving thread state through the existing mesh client. It must also verify the standing-instruction hash on resume. Do not infer either condition from a short reply. This adapter is required future tooling, not code supplied by this design.

Run each follow through the existing detached job route. Start the next operation from its completion notice. Do not poll. Never run both arms at once.

### Order, conditions, and budgets

Use eight A/A pairs of fresh forks. Keep preparation identical. Alternate which temporary thread name runs first. Record the signed first-minus-second difference to detect order effects.

For A/B, use this fixed order by task number:

```text
AB BA BA AB AB BA AB BA BA AB BA AB
```

Start the second arm within two minutes after the first finishes. A later start invalidates the pair. Record elapsed separation and account/server cache evidence. Do not flush a provider cache.

Start only when natedev one-minute load is at most 8. A pair is invalid if unrelated load exceeds 16 during either arm. Record load every five seconds. A changed account, model, tier, tool hash, or input invalidates the pair. A content error or timeout is a task outcome, not a condition exclusion.

Allow at most two replacement pairs across the whole study. Use the next eligible task group for a replacement. Retain the invalid pair. Keep the same assigned order for its replacement.

Stop after eight valid A/A pairs and twelve valid A/B pairs. The deadline is 14 calendar days after registration. The model budget is 1,000,000 weighted tokens, including warmup and invalid attempts. The active model wall-time budget is four hours. Stop at the first limit reached.

Before a new review turn, reserve an allowance of 25,000 weighted tokens from the remaining budget. Refuse a start if that allowance is unavailable. Record actual use afterward. A turn can exceed the estimate; stop after that turn and report the overrun. Do not claim a hard billing limit that the launcher cannot enforce.

### A/A analysis and final rule

Use the framework's paired median and exact 95% order-statistic interval. For A/A, compute N from absolute pair differences. Stop as `not_proven` if N exceeds 2,000 tokens or the signed interval excludes zero.

For A/B, use `d = weighted_A - weighted_B`, including allocated setup usage. Set `T = max(2000, N)`. The twelve-pair interval uses the third and tenth sorted differences.

Accept the limited claim only if all these conditions hold:

- Twelve valid independent pairs exist.
- The lower effect bound exceeds T.
- Every B reply retains the standing instructions and passes every required check.
- No B reply has a confirmed harmful instruction breach or fabricated defect.
- B has no timeout and no missed required defect.
- B's median paired wall-time increase is at most five seconds.
- No B review takes more than thirty seconds longer than its A pair.
- The measured weekly-use calculation below remains positive after setup and reset cost.

Reject B if it violates a correctness or cost limit. Reject the minimum-gain claim if the upper interval bound is below 2,000. Otherwise record `not_proven`.

Keep A's failures as evidence. A failure does not excuse a B failure. The experiment director scores both outputs and must not operate the arm launches. The scorer receives outputs as X and Y. X is A for odd task numbers and B for even task numbers. The scoring packet hides that task number and the thread metadata. The operator keeps the mapping outside the packet. The director records scoring reasons before the labels are revealed. Include scoring usage in the study budget, but exclude it from each arm's primary measure.

If a required defect was absent from the sealed checklist, record it as a secondary discovery. A confirmed harmful B instruction breach still stops the study. Do not turn a new discretionary preference into a required check after the run.

### Live effect and weekly value

This is already a live-task study. It does not need a separate synthetic speed claim. Its accepted scope remains these eligible first reviews, with this model and account setup.

At registration, freeze the preceding seven complete UTC days of eligible review dispatch records. Use phase and role records to identify dispatches. Read each candidate's recorded input size and task type to apply the same eligibility rule. Unknown eligibility stays unknown.

Preserve the selection query and the accepted dispatch IDs. Use `~/.local/state/usage-model/usage.db` and the owning run histories. The usage ledger does not contain every eligibility field. The dispatch manifests supply the missing fields. Sources: `~/.claude/docs/as-built/usage-ledger.md`, `~/.claude/scripts/history/history.py`.

Compute mean savings per review from the twelve pairs. Multiply by the observed eligible weekly count. Deduct one B warmup per observed reviewer-server restart. Do not subtract the already allocated trial warmup twice: use unallocated per-task differences for this weekly calculation.

Report a conservative case with one warmup for every eligible review. Report the observed-restart case separately. Use the minimum and maximum observed unallocated pair savings to show an empirical range around the mean projection. Label it an observed range, not a confidence interval for weekly savings. Require the conservative calculation to stay positive even at the minimum observed pair saving. If the store cannot establish eligible volume, record `not_proven` for weekly benefit.

Report total experiment usage as its own cost. Both-arm review cost is study cost, not future operating cost. Do not count study dispatches in the weekly population. Do not claim user time saved from a token result.

### Records, adoption, and later check

Store protocol, task manifests, both outputs, scorer decisions, conditions, and raw request keys under the experiment record. Keep the warmup record. Do not delete rollouts until their required evidence has a verified durable copy.

The proposed command sequence is:

```text
cargo experiment find --subject process/review-pool
cargo experiment register <sealed-protocol.json>
cargo experiment run start <id> --slot aa-01-first
cargo experiment run record <id> --attempt <attempt-id> --evidence <completion.json>
cargo experiment run start <id> --slot aa-01-second
cargo experiment analyze <id> --stage aa
cargo experiment run start <id> --slot ab-01-A
cargo experiment analyze <id>
cargo experiment decide <id> accepted --basis <decision.json>
cargo experiment adopt <id> --evidence <adoption.json>
```

The listed slots illustrate the sequence. The runner executes every registered slot before final analysis. Completion records arrive from the adapter. The last two commands apply only after their evidence requirements pass.

The subject hook shows the old instruction-loss finding. The start hook checks the pinned conditions. Completion checks detect changed inputs or load. No hook prints success for every review.

For an accepted result, the owner may enable reset only for eligible read-only reviews in that unit. The shared-default change belongs to its showrunner under the existing authority. Record the policy path and hash before `adopt`. Rollback selects fresh fork at the next task boundary. It must not interrupt active production reviews.

The first follow-up is seven days after adoption, with at least twelve eligible completed reviews. Compare their usage and correctness with the registered limits. A retention failure triggers rollback. Insufficient volume gives `check_due`. A changed model, launcher, instruction file, or account setup gives `needs_recheck`.

Stop the private app-server at closure. Verify that its processes have ended. Preserve evidence before removing scratch working files. Record any failed cleanup as an open finding.

### What this protocol cannot establish

It cannot establish a result for another model, provider cache policy, or Mac setup. It cannot prove the absence of rare review errors. It cannot establish an API-dollar saving from weighted tokens. It can support a reversible local policy with a defined follow-up.

The runtime binding and completion adapter still need implementation. Their availability must be checked before registration. This document authorizes no launch.
