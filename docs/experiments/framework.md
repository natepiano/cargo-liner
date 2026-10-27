# Experiment framework

This is a design. It changes no tool or default. The proposed tool name is `cargo-experiment`.

An experiment tests one claim against a fixed control. Record the rules before the first run. Keep all attempts. Measure variation before the comparison. Check the result in normal work. Adopt a change only within the conditions that the evidence supports.

The framework uses the existing build log, usage ledger, and run history. One experiment record joins their evidence. The record also holds the decision, the adoption, and the next check.

## One-page procedure

1. Find prior results for the repository, paths, and process. Read each applicable result and its current state.
2. Name the owner, question, control A, and change B. State one falsifiable claim.
3. Choose one primary measure. Define its unit, start, end, and minimum useful gain.
4. Define correctness checks. Define each permitted secondary cost and its limit.
5. Select the tasks and conditions before you see results. Set the A/A count, A/B count, run order, and deadline.
6. Register the protocol. Seal its input files and analysis rules with hashes. Record the request that started it.
7. Arrange the worktree reservation and machine access. Use the existing unit and build controls.
8. Run A/A pairs. Compute the observed noise floor. Stop if the conditions cannot support the stated claim.
9. Run each registered pair. Seal its scores and apply its stop rules before the next pair. Keep every attempt.
10. Apply the registered analysis once. Report the effect, interval, noise floor, coverage, and correctness results.
11. For a production claim, check the effect in the registered live workload. Estimate daily or weekly gain from observed use.
12. Record `accepted`, `rejected`, or `not_proven`. Name the decision owner and the evidence.
13. Apply an accepted change through its normal work process. Record its exact default, rollback, owner, and next check.
14. Publish the result in the subject index. Link it from the changed code or process. Retain the protocol and evidence.

The commands in section 13 are proposed commands. They do not exist as part of this design.

## 1. Question and hypothesis

Use this form:

> For population P, change B reduces measure Y by at least M compared with A, under conditions C. Correctness checks G must pass.

Name the population in terms of work. Examples are one test invocation, one complete review task, or one completed phase. A model request is not a complete review task.

Specify the following items:

| Item | Required meaning |
| --- | --- |
| Subject | Repository identity, path scopes, and process names |
| Question | The decision that the result will support |
| A | Current behavior, with its source version |
| B | One change, with its source version |
| Mechanism | Why that change can affect the measure |
| Primary measure | One number per independent work item |
| Minimum useful gain M | A number in the primary measure's unit |
| Correctness checks | Required behavior and evidence for each check |
| Scope | Machines, task classes, models, accounts, and cache conditions covered |

Write the failure of the claim as well. For example, an interval that includes gains below M does not establish the claim.

Do not combine a model change, a prompt change, and a review change in one B. Split them into separate protocols. A combined change needs a claim about that combination only.

The ranking is correctness, then simple, fast, and beautiful. A faster incorrect result fails. A faster design with more concepts needs an explicit review of that cost. Source: `~/.claude/docs/decision_criteria.md`.

## 2. Pre-registration

Register one immutable `protocol.json` before any measured run. Section 12 defines its location and fields. The registration event stores its SHA-256, UTC time, owner, and request reference.

The protocol must fix these rules:

- Question, claim, A, B, and primary measure.
- Task eligibility and selection order.
- Independent sample unit and any groups of related samples.
- A/A count and A/B count.
- Run order or the exact seeded allocation algorithm.
- Cache preparation and excluded preparation work.
- Conditions that prevent a start or invalidate an attempt.
- Minimum gain, analysis, confidence level, and correctness limits.
- Replacement limit, time limit, and stop rule.
- Live check, adoption scope, rollback, and follow-up rule.

A future input can have a fixed selection rule. For example, select the next 12 eligible review tasks. Seal each task before either arm sees it. Do not select tasks from their observed results.

Use `awaiting_input_selection` for inputs that the selection rule has not selected. Use `sealed_inputs` after selection. A run cannot start with required inputs still unsealed.

Keep registration, raw attempts, and decisions as separate records. Each attempt names the protocol hash. The journal gives events increasing sequence numbers. An event also contains the previous event's hash. A registration receipt goes in the owning unit's phase record before its first launch. This supplies a separate reference to the protocol hash. A local hash alone cannot prove when an author wrote a file.

Do not edit a registered rule. A necessary change creates a new protocol revision and a new registration receipt. Close the old comparison as `not_proven` if its required data are incomplete. Do not combine results across revisions for acceptance. Label any later analysis of the old data `exploratory`.

The normal stop is the fixed sample count. Stop earlier for a correctness failure or the deadline. Do not stop early because the effect looks good. Do not add runs because an interval almost meets the rule.

Register the purpose as `production_gain` or `framework_exercise`. An exercise uses the same paired method and stop rules. It tests the record and analysis path on a fixed workload. It does not establish production value. Its scope forbids adoption. A completed exercise reports exploratory statistics and closes as `not_proven` for production gain. A confirmed B failure still gives `rejected`.

## 3. Baseline and control

### Code

Pin the common base commit. A contains that base. B contains that base plus the exact change. Record the full commit IDs and any patch hash. A moving branch name is insufficient.

Use separate, owned worktrees or scratch clones. Give them equivalent paths, dependency versions, toolchains, and target state. If path-dependent cache keys matter, include path as a controlled factor. Do not use another unit's working tree.

Prepare each arm by the same rule before each pair. Run adjacent pairs in balanced order: A/B, then B/A. A registered seeded order is also valid. Do not run all A samples before all B samples.

A pair uses the same input and machine. Treat natedev and the Mac as separate populations. Report their results separately. A natedev result does not establish a Mac result.

### Process and model work

A task pair uses the same frozen input, acceptance requirements, tools, and time limit. Each arm receives an independent context. Do not show A's answer or review to B.

Pin the prompt and skill text by content hash. Pin the model ID, effort, service tier, account label, and launcher version. Record context reuse, cache age, and compaction state. A model alias does not promise identical future model behavior.

A must match the current policy in the adoption scope. Verify its command and implementation before registration. Record rollback to that same policy. A comparison against a different policy cannot justify replacing the current default.

The current unit policy resumes its own reviewer without reset. The allocator reuses that unit's idle reviewer. Fresh forks against empty reset threads do not test that default. A retained-context policy study is unsupported in the first version. It needs a design for related observations across reviews. Sources: `~/.claude/commands/unit/direct.md`, `LongLivedWorkers`; `~/.claude/scripts/direct/review_pool.py`, `_unit_reviewers` and `_choose`.

Any later supported thread-reuse study must record the retained UUID. It must verify identity at submission and completion. Replacement or missing identity cannot count as a valid retained-thread observation.

For a review comparison, score both outputs with the same rubric. Hide arm labels from the scorer. Count distinct confirmed defects, missed required defects, and incorrect findings. A longer list of findings is not evidence of better review.

Before registration, the operator assigns X/Y labels with an independent random bit for each pair and replacement slot. Include a secret random 32-byte salt in the mapping file. Register its SHA-256 commitment. Keep the mapping and launch records outside the scorer's access. Labels must not depend on task number or order. If access cannot be separated, refuse registration.

Seal each pair's scores before the operator applies the concealed mapping to its stop rules. Section 6 defines this gate. The operator does not reveal mappings during the study. At closure, seal scores for every collected output before revealing the mapping and salt. Check the commitment before analysis. A changed mapping or early disclosure gives `not_proven`. Automated exact-output checks use label state `not_applicable`; they still require the score gate.

The first tool version supports only same-input paired gain studies. It does not support allocation of different phases to different arms. It also does not support equal-quality or equivalence claims. `register` refuses these designs as `unsupported_design`. Such studies need a separate design review before execution through this tool.

Define dependence before launch. Name each shared cause that can link pair errors: unit history, session, provider cache history, machine date, or reused mutable state. Join candidate pairs that share any declared cause. Take the connected groups, including indirect links. Select at most one pair from each group. A distinct unit-and-date key alone is insufficient: repeated units link dates, and shared dates link units.

The protocol must state how preparation removes a cause, or include that cause in its groups. The admission check compares each new pair with all earlier selected pairs. Refuse a link to an occupied group. If twelve pairs share one evolving thread, they form one group. The first version cannot compute twelve independent observations from it. Refuse such a plan as `unsupported_design` before launch. A newly found link closes an active study as `not_proven`. Do not repair it by choosing favorable pairs. Section 6 of `worked-examples.md` uses separate fresh processes and one pair per machine date.

Before/after history without allocation is observational evidence. A concurrent unchanged control can detect a time trend. It cannot remove every difference between tasks. Do not give that comparison a causal acceptance label.

Sources: `~/.claude/docs/as-built/director-model-study.md`, `~/.claude/docs/as-built/review-regime-trial.md`, and `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/attempts-reader-tests-mac-under-a-second.md`.

## 4. Conditions

Record conditions before launch and after completion. Sample changing machine conditions during a long run. Use a five-second interval for runs longer than ten seconds. The collector must not start builds or query large stores during timing.

| Condition | Record and rule |
| --- | --- |
| Machine | Host, OS, architecture, logical CPU count, power mode, and boot identity |
| Load | One-minute load before, during, and after; keep the maximum |
| Memory | Available memory, swap, and pressure evidence when available |
| Other work | Active build jobs, CI work, and concurrent unit IDs |
| Access | Build hold state, holder, and Mac claim state |
| Build inputs | Toolchain, lockfile, flags, environment allowlist, and target directory |
| Caches | Target contents, sccache server and directory, cache preparation, and hit counters |
| Verification reuse | Actual execution, reused pass, or replayed lint failure |
| Agent | Model, effort, tier, account label, prompt hash, context state, and tool versions |
| Data coverage | Source freshness, missing fields, and collection failures |

Do not store credentials or the full environment. Store the values that can change the result.

For a controlled natedev timing claim, the default start limit is load 8 on 32 logical CPUs. A sample above 16 is untrusted. These are proposed experiment limits. They are not a claim about `verify.sh` admission rules. A held quiet check currently waits for load below one quarter of the cores. Source: `~/.claude/commands/build/hold.md`.

Use the same fractions on the Mac only when no platform-specific limit was registered. Record its measured CPU count. A protocol may set a stricter limit. Do not alter a limit after seeing an effect.

A normal-use study has a different load rule. Register its supported load range from a fixed historical window. Keep high-load work in that population if normal work includes it. Report it by load group. Do not remove slower B runs because B increased load. That increase is an outcome.

Apply the failure table in section 6. Only a measured, registered environmental breach permits replacement. Missing required evidence does not permit replacement. Keep every collected arm and the reason. Report invalid counts by arm and cause. Unequal losses can make a result `not_proven`.

An arm failure is not an invalid environment sample. Preserve its elapsed time and failure outcome. A missing result is not zero. Use `not_collected` or `collection_failed`, with a reason.

Define cold and warm precisely. For example, `empty_private_target` does not mean an empty sccache. Use private cache state for controlled cold runs. Never clear a shared cache to prepare an experiment. For warm runs, define the warmup command and count. Keep warmup cost in the cost record.

`verify.sh` can return stored results without executing the workload. Its cache key includes tree and tool inputs. Record reuse as reuse. Do not count it as another execution sample. Sources: `~/.claude/scripts/direct/verify.sh`, `~/.claude/scripts/buildlog/record.py`.

The build log's machine sampler requires Linux `/proc` files. It records memory, pressure, and power state, but not one-minute load. It cannot supply Mac condition samples. The proposed adapter collects load on each measured host. Section 12 defines the Mac route. Sources: `~/.claude/scripts/buildlog/sample.py`, `sample`; `~/.claude/scripts/buildlog/index.py`.

## 5. Measures and existing evidence

Wall clock is the only time measure used to claim a time gain. CPU time, process counts, and summed task durations can explain a result. They cannot replace elapsed time. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/reduce-test-load.md`.

| Experiment | Usual primary measure | Secondary measures and correctness evidence |
| --- | --- | --- |
| Build or lint change | Complete invocation wall seconds | Failures, diagnostics, rebuilds, process starts, cache reuse |
| Test change | Full relevant suite wall seconds | One-test wall seconds, coverage, failures, retries, CI result |
| Model, prompt, or skill | Wall seconds per completed task, or weighted tokens per assigned task | Confirmed defects, missed requirements, cost, corrections, repair rounds |
| Review regime | Remaining confirmed defects on the same frozen review task | Review wall time, reviewer cost, incorrect findings; phase-policy claims need a separate design |
| Application change | Wall latency for a named user action | Correct output, completion rate, resource use |

Choose one primary measure from the appropriate row. Define quality and cost limits separately. Count a timeout or abandoned task in the quality result. Do not report only successful tasks.

For an end-to-end time claim, start at the request and end at the usable result. Include queues, copies, and necessary repairs. A controlled execution-only measure may exclude queue time, but its name must say so. Record queue time separately. Use a monotonic clock for elapsed time. Use UTC for event joins.

| Source | What it supplies | Query route and limits |
| --- | --- | --- |
| `~/.claude/scripts/buildlog/store.py`, `index.py`, `cli.py` | Host records, calls, steps, tests, CI, and a derived index | `buildlog schema`; `buildlog query --json '<SQL>'`; preserve SQL and window |
| `~/.claude/commands/build/report.md`, `mac_report.md` | Human build reports and Mac outcome coverage | Reports are on demand; keep host coverage separate |
| `~/.claude/docs/as-built/usage-ledger.md`; `scripts/usage/usage_report.py` under `~/.claude/` | Requests, token classes, roles, phases, passes, findings, and source states | `usage_report.py list`; named metrics such as `weighted` and `wait_share` |
| `~/.claude/scripts/usage/weights.py` | Dated weights by model | Save the weight version with the result |
| `~/.claude/scripts/history/history.py` | Stage durations and phase sums of stage time | `phases --status all --format json`; `total_seconds` is a sum, not phase elapsed time |
| `~/.claude/scripts/usage/collect.py`; `~/.claude/docs/as-built/usage-ledger.md` | Phase start and finish events and their ledger projection | Join `phase_started` and `phase_finished` by machine and `phase_instance_id`; check the run ID |
| `~/.claude/scripts/production/review_regime.py` | Phase review outcomes | `report --since <date>`; preserve ledger rows and their field meanings |
| `~/.claude/scripts/model_study/model_study.py` | Director request comparison and phase analysis | Read saved results; its commands also write derived files |
| `~/.claude/scripts/nightly_review/candidates.py`; `~/.claude/commands/efficiency_next.md` | Measured proposals and adoption outcomes | Link the experiment from the existing proposal line |

The build log stores permanent JSON lines under `~/.local/state/buildlog/<host>/`. Its `index.sqlite` is derived. `buildlog query` updates that index before its read-only SQL query. For a strictly read-only investigation, open the existing index with SQLite `mode=ro`. Record its freshness. Sources: `~/.claude/scripts/buildlog/store.py`, `cli.py`.

The usage ledger is `~/.local/state/usage-model/usage.db` on natedev. Its report opens a read-only snapshot. Save source coverage with each query. A missing Mac read is not evidence of zero Mac use. Sources: `~/.claude/docs/as-built/usage-ledger.md`, `~/.claude/scripts/usage/usage_report.py`.

For phase elapsed time, subtract the matched start timestamp from the finish timestamp. Keep both source events. The ledger's `phases.started_at` and `finished_at` are a query route to those events. A missing boundary or conflicting event gives `collection_failed`. Check timestamp order and clock changes. Stage sums can include overlap and omit gaps. Do not use them for a phase time gain. Sources: `~/.claude/scripts/history/history.py`, `_phase_rows`; `~/.claude/scripts/usage/collect.py`, `HISTORY_FIELDS` and `save_history_event`.

Codex ledger keys name source locations. `codex_key` returns `filename:byte-offset`. A request copied into another rollout gets another key. These keys alone cannot establish unique execution. A usage adapter must separate new execution from replay and retain every source location. Ambiguous attribution gives `collection_failed` and prevents acceptance. The first version does not support rollover or resume during a measured turn. It also does not support Codex usage as a primary measure until an adapter can prove new execution from serving-thread and turn evidence. Existing parser keys do not meet that requirement. The first live protocol therefore uses command wall time. Sources: `~/.claude/scripts/usage/collect.py`, `codex_key`; `~/.claude/scripts/usage/rollouts.py`, `token_usage`.

Weighted tokens, API-equivalent dollars, and subscription use are different measures. Do not convert between them without a stated model. Unpriced usage has state `unpriced`. Do not assign it zero cost. The director study's price table and the usage ledger's weights serve different purposes. Sources: `~/.claude/scripts/model_study/stats.py`, `~/.claude/scripts/usage/weights.py`.

No inspected general store supplies a complete process-start count or a verified user-correction count. Collect these in the experiment when required. Name the trace or correction event. Deduplicate defects by cause and affected behavior. Keep scorer decisions with source references.

Review-pass count and repair-round count differ. The token study found unusable `fix_pass` values for its review analysis. Read the phase events for the required count. Sources: `~/.claude/docs/as-built/token-efficiency-study.md`, `~/.claude/scripts/history/history.py`.

## 6. Repeat runs and noise floor

### One supported analysis

This analysis applies only to same-input pairs with one independent work item per pair. Its claim is a positive gain above M. Unpaired allocation, group estimates from several related pairs, and equal-quality margins are unsupported in the first version. The tool must refuse those protocols before launch. It must not compute a paired verdict for historical unpaired data.

Use paired differences. For a lower-is-better measure, define `d = A - B`. Positive d means an improvement. Reverse the sign for a higher-is-better measure.

Use the median of independent pair differences as the primary effect. Do not subtract two unpaired medians and call it a paired result. Show every pair beside the summary.

The default protocol has eight A/A pairs and twelve A/B pairs. These are starting counts, not a promise of sufficient evidence. Choose larger fixed counts before registration when the effect is small or the risk is high. State the precision reason.

For expensive agent work, use whole tasks as pairs. Six to eleven independent pairs can support only large, consistent effects. Fewer than six pairs give no finite two-sided 95% median interval under this rule. Report them as exploratory and `not_proven`.

### A/A stage

Run the same baseline through both preparation paths. Apply the planned order, caches, tasks, and collectors. A/A work must exercise the variation expected in A/B work.

Let `a_j = A_first - A_second`. The observed noise floor N is the nearest-rank 90th percentile of `abs(a_j)`. With eight pairs, N is their maximum. Report the signed median and range as well. This is an observed limit, not a population guarantee.

Stop if the A/A median interval excludes zero. The preparation paths or time trend need investigation. Stop if N exceeds M. A more precise study needs a new protocol, not a larger sample chosen after B appears.

Do not include A/A observations in the A/B effect.

### Interval and decision threshold

Use an exact order-statistic interval for the median. This avoids a narrow bootstrap interval from a few agent tasks.

Sort the n differences from smallest to largest. Choose the largest integer k for which:

`2 * sum(C(n, j), j = 0..k-1) / 2^n <= 0.05`.

Here `C(n, j)` is the binomial coefficient. Limit k to `1..floor((n+1)/2)`.

The interval is `[d[k], d[n-k+1]]`, with one-based indices. If no k exists, the interval is `unbounded`. Ties make this rule conservative. With twelve pairs, the bounds are the third and tenth differences.

This interval assumes independent pairs from the stated population. Random order reduces time bias. It does not make related tasks independent. If the required independent groups are absent, report `not_proven`.

Set the acceptance threshold T to `max(M, N)`. A gain claim requires the lower interval bound to exceed T. Keep absolute units as the decision measure. Percentages are secondary unless the protocol fixed paired percentage change as primary.

For daily or weekly totals, also report arithmetic mean savings within each workload class. A median gain cannot be multiplied by all daily calls to estimate total time saved.

The director study provides a seeded bootstrap implementation for larger historical comparisons. It resamples requests independently. Do not reuse that implementation for paired or clustered tasks without changing its sampling unit. Source: `~/.claude/scripts/model_study/stats.py`.

### Score gate between pairs

A workload completion records execution evidence. It does not establish that correctness checks passed. Keep completion receipts immutable. Append scores and stop decisions as separate events.

Use this sequence for each pair:

1. Import the pair result and the receipts for executed arms. Retain prevention evidence for any unexecuted arm.
2. Give the scorer the sealed inputs, rubric, and X/Y outputs. Omit arm identity, launch order, and timing metadata.
3. Have the scorer seal the scores and reasons with a hash. Retain the scorer's receipt before the operator reads them.
4. Have the operator import those scores with `run score`. The tool checks their hashes and the registered mapping commitment. It applies the stop rules with the concealed mapping.
5. Append the score event and gate decision in one journal transaction. Permit the next pair only for `continue_permitted`.

Keep the mapping and arm-specific gate evidence in the operator's private record until closure. The scorer receives no arm identity. A stop closes data collection before the final reveal. All collected scores must already be sealed. A scorer cannot revise a score after mapping or timing evidence appears.

For automated checks, `run score` imports the sealed checker's result and applies the same rules. Record the checker version. Scores not yet delivered leave `awaiting_scores` and block the next pair. A confirmed loss or collection failure follows the failure table below. At the deadline, absent scores give `not_proven`. The pair adapter checks directly observable terminal conditions between arms. `run record` also blocks later starts when it imports a terminal condition. Do not finish another arm merely to fill the pair. Seal scores for collected outputs before final disclosure. Keep those score events separate from the completion evidence.

### Failures and replacements

This table is the single failure rule for preflight, adapters, imports, scoring, and analysis. Apply it before selecting another pair.

| Case | Required evidence and action | Replacement and decision |
| --- | --- | --- |
| Preflight refusal before either arm starts | Known adverse conditions or unreadable required conditions; record `not_started` and launch nothing | No replacement consumed; retry only after a condition change or an explicit action; deadline still applies |
| Measured environmental breach after the pair starts | Complete required measurements prove a registered unrelated breach, such as load or arm separation; stop before any remaining arm | Mark the pair `invalid`; replace the whole pair only within the registered allowance |
| Missing required measurement | A required condition, primary value, source field, or sealed score is confirmed lost, unreadable, or invalid | Stop as `not_proven`; no replacement; retain trust `unknown` and the collection failure |
| Missing or conflicting workload receipt | Execution cannot be reconciled from the fixed receipt path and launch association | Record `execution_unknown`; block all later starts; recover retained evidence without execution under section 12; unresolved at closure gives `not_proven` |
| Workload correctness failure | Retained evidence confirms the failure, including a registered timeout | B in A/B or live work gives `rejected`; A or A/A gives `not_proven`; stop and do not replace |
| Other terminal limit | Deadline, changed source binding, new dependence, or incomplete pair without a registered environmental cause | Stop as `not_proven`; no replacement |

An in-flight receipt or score is not yet missing evidence. Keep its waiting state and block the next pair. A source known to be unavailable cannot be treated as still in flight. Recovery imports the original evidence; it does not select a new observation.

If cases overlap, a confirmed B correctness failure takes precedence. Otherwise missing evidence takes precedence over an environmental breach. Do not attribute a failure to B from corrupted inputs alone. A B-caused load increase is a measured outcome, not an unrelated breach. Apply its registered cost rule. A breached B cost limit gives `rejected`.

Register at most two replacement pairs by default. A replacement requires complete evidence of the registered environmental cause. It does not count toward the valid-pair target. Keep incomplete environmental pairs and skipped arms in the result table. A refusal before the second arm is a started-pair case, not a new preflight exemption. Exhausted replacement allowance gives `not_proven`. Investigate an A or measurement failure before a new registration. Never replace a task failure with a successful retry.

## 7. Live effect and daily value

A controlled result establishes an effect only for its measured conditions. It does not establish a useful gain in production.

Register a live check before the controlled comparison. Define the task mix from a fixed store window. Include failures, fixes, cache states, repeated calls, and inactive callers. Keep task selection independent of the arm's result.

A `framework_exercise` has `live_check` state `not_claimed` and frequency coverage `not_required`. Give the reason. Do not create a recurring workload or frequency estimate to obtain acceptance. It cannot pass the production acceptance rule in section 8.

Verify every required eligibility field against its producer, schema, and retained path. A query cannot supply a field its writer never recorded. If coverage is missing, register a prospective baseline window before A/A. Keep its collector version, manifest fields, daily coverage, exclusions, and no-use days. Freeze that window before comparison. Do not invent historical dispatch manifests. For example, `review.sh` records review identity, timing, kind, and outcome. It does not write complete input sizes or eligibility manifests. Source: `~/.claude/scripts/direct/review.sh`, `write_turn_record`.

Use the same-input paired method on eligible live tasks within one reversible unit. If live work cannot run as independent pairs, report its evidence as observational. The first version cannot accept a gain claim for that population. An owner may authorize a limited pilot separately. That action does not change the evidence verdict.

Estimate weekly total savings by workload class:

`weekly saving = sum(observed calls per week * mean saving per call) - added weekly operating cost`.

Keep setup, installation, maintenance, and rollback costs in a separate table. State the payback period if that informs the decision. Do not assign a recurring gain to a caller with no observed use.

Freeze the frequency window before analysis. Save the SQL, source rows, host coverage, and time zone. Use low and high frequency cases from the observed weeks. Keep uncertainty in the saving estimate. An unmeasured case contributes an unknown amount, not an assumed saving.

Summed agent waiting time is not user elapsed time. Concurrent calls overlap. Report user waiting only from observed request-to-result intervals or the production's critical path. Do not add overlapping intervals twice.

The mend and clippy experiment projected about one hour per day from selected runs. Its later live estimates were 5–25 minutes per day, with ranges that included zero. Failure and fix runs changed the result. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/handoff-mend-clippy-one-compile.md`.

The token-efficiency study labels replay projections as modeled. It separates exposure from savings and records zero observed live savings. Apply that distinction to every replay. Source: `~/.claude/docs/as-built/token-efficiency-study.md`.

## 8. Acceptance and decision owner

The analysis records evidence. The decision owner records the action. Neither step changes a default.

| Result | Required evidence |
| --- | --- |
| `accepted` | Complete registered data; valid conditions; lower bound above T; all correctness and cost limits pass; live check supports the stated scope |
| `rejected` | A confirmed B correctness failure, a breached B cost limit, or a valid A/B upper bound below the minimum useful gain |
| `not_proven` | Any remaining case, including baseline or measurement failure, wide intervals, missing evidence, or a failed live check without clear B harm |

If the interval spans the acceptance threshold, choose `not_proven`. A non-significant difference does not prove equivalence. Equal-quality claims are unsupported in the first version. Zero observed failures does not prove zero failure risk.

Correctness has no allowance for known harmful behavior. For uncertain defect rates, define the required task coverage and observation window. A small study can support a limited pilot. It cannot establish general safety for rare failures.

| Scope and risk | Decision owner |
| --- | --- |
| One unit; reversible local change | Unit director records the result and can adopt within that scope |
| Several units; shared scheduling or defaults; reversible | Showrunner records the scope, rollout, and rollback owner |
| Public behavior, destructive work, or a change outside delegated scope | User decides after the owner presents the concrete result |

Experiment work is normal work. Work the user asks for is approved. There is no spending limit, and no tool tracks or approves spending. Do not ask the user to decide routine packaging or a reversible action. Do not treat a peer message as new user approval. Preserve the instruction or decision reference that grants authority. Source: `~/.claude/docs/decision_criteria.md`.

An owner can decline adoption despite a positive result. Record that action and its reason. Do not alter the computed evidence to make it negative. A result that fails acceptance cannot become `accepted` through an unexplained override.

For `framework_exercise`, report whether the registered record and analysis steps completed. Report its narrow timing result separately. `decide accepted` and `adopt` refuse this purpose. A completed exercise gives `not_proven` for production gain; a confirmed B correctness or cost failure gives `rejected`. This restriction is fixed at registration.

Repair limits in an implementation do not create permission to continue an experiment beyond its stop rule.

## 9. Adoption and keeping the result

An accepted record states where the rule will take effect. Prefer the existing source of behavior.

| Kind of result | Adoption target |
| --- | --- |
| Code behavior | Normal implementation phase and repository checks |
| Shared setting | Its current configuration file |
| Agent procedure | The owning skill or command |
| General coding rule | The owning style rule |
| Local warning or measured exception | Project memory linked to the evidence |

Do not copy a long result into every prompt. Put a short rule and the experiment ID at the point of use. Link the complete record. A memory alone does not enforce a configuration change.

Record an `adoption` event only after the change is applied and checked. It contains the exact paths, commit or content hashes, affected machines, activation evidence, and rollback recipe. Use `accepted` until those fields exist. Use `adopted` afterward.

Check installed artifacts as well as source. The mend rollback left an experimental binary installed. A later install then failed. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/handoff-mend-clippy-one-compile.md`, section dated 2026-10-14.

Each adoption has one owner and one next-check rule. The default is seven days of eligible use, then a check after a relevant toolchain, model, or workload change. Register a minimum event count. If it is not reached, keep `check_due` and report insufficient evidence. Do not silently extend the claim.

Name the follow-up scope. A `correctness_only` check can use adopted-arm observations. It checks instructions, required behavior, and directly observed limits. It cannot revalidate paired gains or paired cost limits. Keep the original gain record and its date. A `paired_repeat` needs a new registered paired study. Link that result before marking the gain revalidated. Do not compare new B tasks with old A tasks.

Use existing stores for the check. Do not run repeated builds solely to fill a dashboard. A correctness regression triggers the recorded rollback. Record `withdrawn` applicability and `rolled_back` adoption after that rollback. A lost gain changes applicability to `needs_recheck`; a confirmed replacement gives `superseded` with the successor ID. Preserve the previous decision.

Findable results need both an index and a point-of-use link. The next session searches the index before it registers another experiment. Section 13 defines the lookup hooks. A result whose conditions no longer match is a reference for a new study, not a current default.

Link existing nightly proposals to the result. Keep the ledger's original line and update its outcome through the existing process. Do not delete rejected proposals or revive them without new evidence. Sources: `~/.claude/commands/nightly_review.md`, `~/.claude/commands/efficiency_next.md`, `~/.claude/docs/as-built/token-efficiency-study.md`.

## 10. Repeatability

A repeat must recover the experiment's inputs and analysis without reading this conversation.

Keep these artifacts:

- Protocol, registration receipt, and all revision links.
- Full base and arm commit IDs; patches and untracked input hashes.
- Exact argument arrays, working directories, and environment allowlist.
- Dependency lockfiles, tool versions, prompts, skills, and scorer rubric.
- Ordered input manifest and allocation seed.
- Preparation, warmup, cache, load, hold, and machine records.
- All attempts, outputs, exit codes, timeouts, and source row keys.
- Frozen source extracts with hashes and coverage.
- Analysis version, formulas, output, and decision reference.
- Adoption evidence, rollback, and next-check record.

Copy the minimum raw evidence needed before temporary sources disappear. A `/tmp` path alone is not retained evidence. Keep full private transcripts outside published summaries. Save task inputs and outputs in the private experiment directory when required for repeatability.

Record store keys as well as file paths. Build records need host and call or step ID. Model requests need machine, family, agent identity, source location, and evidence of new execution. A Codex ledger key is a source location, not a logical request ID. Preserve an extracted snapshot because later collection can repair attribution. Sources: `~/.claude/scripts/buildlog/index.py`, `~/.claude/docs/as-built/usage-ledger.md`, `~/.claude/scripts/usage/collect.py`.

Observations on later dates remain in the same study when its registered sampling plan requires them. Do not split that sample at midnight. A repeat starts only after the registered study closes. Create a new record linked to the closed study. Use the same protocol rules with new observations and a new registration receipt. Compare intervals and noise floors. If a model version or workload changed, label the result a new population. Do not promise byte-identical agent output. Require the same decision within the registered uncertainty.

If evidence is lost, set `evidence_unavailable`. Keep the historical conclusion visible, but do not present it as repeatable.

## 11. Place in a production

An experiment is a bounded phase in an existing unit. Its Work Order names its record, owner, files, machines, sample counts, stop rule, and report path. A shared process experiment belongs to the showrunner that owns the process. Source structure: `~/.claude/docs/production_format.md`, `~/.claude/commands/unit/direct.md`, `~/.claude/commands/showrunner/produce.md`.

Use the unit's phase reservation state for edits. Do not create a second reservation for the same phase. Berth protects paths and integration order. It does not provide a quiet machine. Sources: `~/.claude/commands/unit/direct.md`, `crates/cargo-berth/README.md`, `docs/cargo-berth/as-built/operations.md`.

Prefer an existing quiet period. If a controlled timing run needs a build hold, state its measured end time. Use the existing hold procedure. Release it after success or failure. Confirm release once. Do not edit another session's hold files.

The old hold-release memory records a release that did not finish. The current command documents a bounded release process. Use the current procedure and verify its result. Sources: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/build-hold-release-may-never-finish.md`, `~/.claude/commands/build/hold.md`.

Start `verify.sh` calls one at a time. Start the next call only after the completion notice. Never use simultaneous filtered runs to estimate one test's time. Sources: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/verify-runs-one-at-a-time.md`, `attempts-phase8-berth-tests-under-a-second.md` in the same directory.

Use `verify.sh` for the normal gate. A full Mac pass ends that gate. Do not follow it with a natedev rerun. `--local` requires a test that cannot pass on macOS. A platform timing study needs its own registered platform work. Sources: `~/.claude/scripts/direct/verify.sh`, `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/mac-pass-ends-the-test-gate.md`.

`verify.sh test` and `mac_run.sh` claim Mac access themselves. A manual remote job takes its claim from natedev. A repository claim does not exclude another repository's work. For quiet timing, account for that work or obtain the required block through the owner. Mac blocks can affect CI. Record and resolve skipped CI work at release. Source: `~/.claude/commands/build/mac.md`.

Do not add multi-second tests as measurement infrastructure. Record each slow test as a finding. Preserve major test paths and CI behavior. A product-imposed timing floor needs the user's existing exception or a new decision. Sources: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/no-multi-second-tests.md`, `reduce-test-load.md` in the same directory.

Do not run experiment loops while other units collect timings on the same machine. The showrunner assigns the measurement window. Long commands use the current detached runner and completion notices. Do not poll or start a second scheduling system.

## 12. Records and tools

### One experiment directory

Proposed owner-store root: `~/.local/state/experiments/` on natedev. Each experiment lives in `records/<experiment-id>/` under that root. The ID is a UUID. The record names one owning repository and its other subjects. Process subjects can name a skill, model policy, or command without a code path.

All mutations go through that owner store. Mac work sends its completion evidence to the natedev launcher. An unavailable owner prevents new recorded runs. It does not permit a second writable copy. This design adds no cross-machine merge protocol.

| File | Owner and content |
| --- | --- |
| `protocol.json` | Immutable registered rules and input selection method |
| `events.ndjson` | Append-only attempts, analysis, decisions, and adoption events |
| `evidence/` | Immutable content-addressed inputs, outputs, scores, and source extracts |
| `workload-results/<attempt-id>.json` | Write-once workload result receipt, saved before the runner can send its notice |
| `pair-results/<pair-id>.json` | Write-once arm receipt hashes and any arm prevented from starting, with its stop evidence |
| `result.md` | Generated report with evidence links and applicability |

Use one owner-store lock for all journal and index transactions. The publication contract below defines its durable order. Never hold it while a job runs. An event has `schema_version`, `event_id`, `transaction_id`, `sequence`, `at_utc`, `protocol_sha256`, `previous_event`, and a tagged `event`. `previous_event` is `journal_start` or `prior_event` with its hash. Completion events have an idempotency key from the launcher and attempt ID. A repeated identical completion changes nothing. A conflicting completion is refused.

Check for active or unresolved attempts under that same lock. Check the previous pair's score gate and any study stop decision. Reserve both arm slots and append one pair launch intent in one transaction. A frequency-baseline launch reserves its single slot. Complete the durable journal and index publication before calling the launcher. The intent binds a launch ID, reserved attempt IDs, a unique launcher label, exact command hashes, and the launcher's record directory. Call the launcher at most once for that intent. A repeated `run start` returns the existing association. It never repeats the launch call.

For `mesh_runs`, pass the launch ID in the unique `--label`. Its saved record includes that label and the command before it spawns the supervisor. Thus the saved record can identify the reserved attempts even if the CLI loses the returned run ID. After launch, append the run ID and record path to the association. Source for the existing record order: `~/.claude/scripts/agents/mesh_runs.py`, `command_start`.

The launch intent fixes each workload-result path and the pair-result path. The proposed execution adapter runs as the runner's child. For a pair, it runs both arms sequentially in registered order within that one job. It measures the gap itself. It does not wait for an agent dispatch or receipt import between arms. It collects each arm's exit, elapsed time, conditions, and output hashes. Seal that arm's receipt before starting the next arm.

Between arms, the adapter checks required measurements, conditions, timeout, exit, and any registered direct correctness checks. Apply the section 6 failure table. On a terminal or environmental stop, record the second arm as `prevented_from_starting` with its cause. Do not create a workload receipt for an unexecuted arm. A second-arm preflight breach cannot be retried inside that pair. Enforce the registered gap before starting that arm. Seal the pair result with each arm's receipt hash or prevention evidence. Human scores still wait until the pair ends. Only `run score` permits another pair.

Flush receipts, the pair result, and their evidence to storage. Publish them atomically without replacement. Flush their parent directories before returning to the runner. This is required for successful, failed, and stopped pairs. A single frequency-baseline attempt uses the same receipt rule without a pair result.

The receipt binds the experiment, protocol hash, attempt, workload command, host, and boot identity. It holds the named outcome, start and finish evidence, monotonic elapsed time, and evidence manifest hash. Its workload exit is separate from the adapter's exit. The journal imports the receipt hash. Duplicate identical imports change nothing. Conflicting receipts stop the study.

`mesh_runs` sends its completion notice before it saves its own finished state. This ordering is deliberate. Do not use that finished state as the workload-result source. The adapter's receipt must already exist when the runner can send its notice. Thus normal delivery can still show a running launcher record while the workload result is complete. Source: `~/.claude/scripts/agents/mesh_runs.py`, `command_supervise`.

Recover through `run record` after a notice or an explicit recovery action. Read the saved launch association and fixed result paths once. Require exactly one matching association and a valid receipt for each executed arm. A pair also needs its sealed pair result. Import these records even if the runner's delivery state is still running. A lost notice does not require another execution. A saved runner exit alone cannot replace receipts or required measurements. No polling or sleep is needed to resolve the normal delivery order.

An absent or conflicting receipt gives `execution_unknown` unless other retained evidence establishes what executed. Block that slot and all later serial slots. A missing process alone does not prove that work never ran. To record `not_started`, prove that the workload never started and that the original starter cannot launch it later. If the owner proves completion, import the retained receipt. Otherwise close as `not_proven` and retain the unresolved execution record. Closure does not permit relaunch.

The adapter can fail before it seals the receipt. The runner can also fail to start the adapter. Neither case permits an automatic retry. `mesh_runs` creates a new ID on each start. Keep the launch intent for reconciliation. Other routes need the same result-before-notice guarantee or are unsupported for experiment measurements. A command that only detaches another job is not a completed workload. Its adapter must receive that job's durable result. Source for the existing start behavior: `~/.claude/scripts/agents/mesh_runs.py`, `command_start`.

### Mac execution and receipt transfer

Keep Mac measurements supported. Reuse the claim and remote-status handling in `~/.claude/scripts/production/mac_run.sh`. Its `on_mac` reads an explicit `mac_exit=<code>` line because SSH status can report zero for a failed remote command. Its Terminal launch also writes the workload status to a remote file. The script is specific to Hana; reuse those mechanisms in the experiment adapter, not its fixed workload or temporary filenames.

Take the existing claim from natedev with `--remote <host>`. Give the remote adapter a durable, attempt-owned directory under `~/.local/state/experiments/execution/<launch-id>/` on the Mac. Reserve a write-once launch guard there before execution. Bind it to the protocol, pair, arm IDs, command hashes, and source commits. Run the pair as one remote job that survives SSH loss. Repeated submission of the same launch ID must not execute work again. Source for remote claims and kept-claim recovery: `~/.claude/commands/build/mac.md`.

Measure each arm's monotonic elapsed time on the Mac. Collect its before, during, and after conditions there. The proposed Mac collector reads load, CPU count, and boot identity through Darwin `sysctl`; memory through `vm_stat` and swap counters; power state through `pmset`. Pin command arguments, units, and collector version at registration. Retain raw readings and their timestamps. Use `not_applicable` for Linux pressure counters, with the platform reason. A required Mac reading that fails follows section 6. Read natedev claim and hold evidence separately. Do not use natedev load or SSH duration as Mac execution time. The existing buildlog sampler is Linux-only: `~/.claude/scripts/buildlog/sample.py`, `sample`.

The Mac adapter records the actual workload exit in each immutable receipt. Its remote shell prints the explicit status line after sealing the pair result. A missing or malformed line is a transport failure, never workload success. Copy the sealed result and referenced evidence back once after normal completion. Verify identities and hashes before publication on natedev. Import them in one durable owner transaction. Only then acknowledge the receipt hashes to the Mac. Write the acknowledgement in the same remote execution directory. Keep remote evidence until that acknowledgement; local imported evidence follows normal retention. The natedev adapter returns to its launcher only after transfer succeeds or it records the unresolved transport outcome. No second writable decision store exists.

After connection loss, set `execution_unknown` on natedev and block later pairs. An explicit recovery reads the same remote launch guard and result paths once. Copy any completed result without re-execution. A repeated copy or acknowledgement with the same hashes changes nothing. Conflicting hashes stop as `not_proven`. If no sealed result exists, retain the claim and unresolved state until the existing remote recovery process proves the job ended. Do not infer completion from a vanished SSH process. Release the claim only after remote work has ended. Sources for existing status and claim behavior: `~/.claude/scripts/production/mac_run.sh`, `on_mac`; `~/.claude/commands/build/mac.md`, State and recovery.

### Index publication and freshness

The owner root holds derived `index.json` and authoritative `publication.json`. The index maps subjects to record IDs, states, and report paths. The publication record has `published` or `updating` state. A published generation binds the index hash and every journal head by record ID, last sequence, and last event hash. Initialize generation zero with an empty index and head map under the owner-store lock. Record creation and applicability changes require a new generation.

All writers use the same exclusive owner-store lock. Publish each journal transaction in this order:

1. Write `updating` with the prior generation, next generation, and transaction ID. Replace and flush `publication.json` and its directory before changing any journal.
2. Append and flush the journal transaction. Retain its transaction ID and complete event boundary.
3. Build the index from those committed journal heads. Include the next generation and exact head map. Replace and flush the index and its directory.
4. Publish `published` with that generation, index hash, and head map. Flush the record and directory before releasing the lock or reporting success.

This lock serializes different experiment writers and index repair. A crash after a withdrawal reaches the journal leaves publication `updating`. The old index remains `stale` even if its JSON is valid. No writer may skip publication invalidation. No later mutation proceeds through an unresolved publication.

Lookup takes a nonblocking shared owner-store lock. It reads the publication record and index under that lock. Return `current` only for `published` with matching generation, index hash, and head map. Return `stale` for `updating`, a busy writer, or a generation mismatch. Return `unavailable` for unreadable state, a missing file, or a hash failure. Release the lock after copying the checked result. Freshness means current at that read. Recheck publication on every lookup, including cache hits. A timestamp alone proves nothing.

`repair-index` uses the exclusive lock and the same publication sequence. It validates journal chains and complete transaction boundaries before rebuilding. It does not drop a committed event or repair an incomplete journal transaction. Such damage returns `unavailable` for owner reconciliation. A failed index publication keeps the store stale until repair succeeds. `decide` and `adopt` validate current journals under this lock; they never take authority from an index entry.

Hooks and the app consume this checked projection. Stale rows may be shown only as historical evidence with their generation. They cannot supply defaults or establish that no prior result exists. Mac mirrors are historical snapshots: their timestamps and hashes do not prove current owner state. A fresh owner lookup through the launcher supplies a dated answer for that action. Offline hooks must label the mirror `stale`. Do not add a database or a second writable index.

Back up the authoritative root with the owner's durable data. Before adoption, archive the sealed protocol, final report, and evidence manifest in the owning repository's `docs/experiments/results/<id>/`. This is a fixed export, not a second writable record. Its hashes must match the owner record. Keep raw evidence for the life of an adopted result. Keep the raw evidence of a result that is not adopted for 90 days after closure, then remove it. The journal, protocol, report, and evidence manifest stay.

### Protocol fields

| Field | Required content |
| --- | --- |
| `identity` | ID, title, owner, repository identity, path and process subjects |
| `authority` | Scope and the request that started the experiment |
| `purpose` | `production_gain`, or `framework_exercise` with its learning goal and adoption prohibition |
| `claim` | Question, mechanism, population, A, B, primary measure, M |
| `inputs` | Sealed inputs or a fixed future selection rule |
| `execution` | Argument arrays, working directory rules, preparation, collection, and completion adapter |
| `conditions` | Machine rules, load rules, holds, caches, model, account, and tools |
| `sampling` | Independent pair unit, group selection, counts, order, replacement limit, and deadline |
| `analysis` | Supported paired gain method, direction, formula, interval rule, noise rule, and limits |
| `correctness` | Required checks, scorer rubric, concealed label commitment, observation window, and arm-specific stop conditions |
| `live_check` | `required` with workload window, producer, fields, coverage, allocation, measures, and scope; or `not_claimed` with the exercise reason |
| `adoption_plan` | `planned` with target, decision owner, rollback, and follow-up; or `not_permitted` for a framework exercise |

### Attempt fields

Each measured arm has one attempt record. A pair joins two attempt IDs. A replacement pair gets new IDs and references the invalid pair. A prospective frequency-baseline execution has its own attempt without a comparison pair.

| Field | Required content |
| --- | --- |
| `identity` | Experiment ID, protocol hash, slot, and attempt ID |
| `assignment` | `comparison_arm` with pair ID, stage, and arm; or `frequency_baseline_observation` with date and control command |
| `work_item` | Input hash, eligibility evidence, task or group ID, and source commit |
| `execution` | Sealed command ID, actual arguments, directory, host, and named launch association |
| `conditions` | Before, during, and after observations with timestamps and source states |
| `outcome` | Named attempt state; workload exit, timeout, or interruption evidence |
| `trust` | Named trust state and every condition violation |
| `measurements` | Primary value, secondary values, units, collection states, and usage attribution |
| `evidence` | Workload receipt, content hashes, source locations, and output paths |

A slot names one arm, such as `ab-03-B`. The protocol fixes its pair and order. A pair launch reserves both slots; separate arm launches are unsupported. The pair result names each arm as `executed` with its receipt hash or `prevented_from_starting` with stop evidence. Each pair has separate score events and a gate state. Scores name the pair, output hashes, rubric hash, scorer or checker version, and sealed decisions. The gate binds those score hashes to the registered stop rules. The board counts a valid pair only after valid arm records and its score gate exist. A condition-invalid pair still needs a gate decision before its registered replacement. That gate preserves the invalid reason and consumes the replacement allowance. A partial pair at a terminal stop cannot count as complete.

### Named states

Use tagged records. Name each field and state for its role or guarantee. Do not use an empty string, absent field, or `null` for a domain state. This applies to launch recovery and usage attribution as well as decisions. Source rule: `~/.claude/docs/type_design.md`.

| Record | States and required data |
| --- | --- |
| Experiment lifecycle | `registered`; `running`; `accepted`; `rejected`; `not_proven`; `adopted`; `superseded` |
| Running experiment | Stage `frequency_baseline`, `aa`, `ab`, or `live`; progress `ready`, `active`, or `paused` with a reason |
| Attempt | `prepared`; `not_started` with evidence; `launch_pending` with durable intent; `running` with launcher evidence; `execution_unknown` with unresolved evidence; `completed`; `failed`; `interrupted` |
| Launch association | `not_requested`; `intent_recorded` with launch ID, reserved attempt IDs, label, command hashes, record directory, and result paths; `launcher_recorded` with those fields, run ID, and record path |
| Workload result | `awaiting_receipt`; `sealed_receipt` with path and hash; `receipt_unavailable` with missing or conflicting evidence; `not_expected` with proof that the arm never started |
| Mac receipt transfer | `not_applicable` for local execution; `awaiting_transfer` with remote result path; `awaiting_acknowledgement` with imported hashes and owner event; `acknowledged` with those hashes and the remote acknowledgement path |
| Index publication | `updating` with prior and next generations and transaction ID; `published` with generation, index hash, and committed journal head map |
| Lookup freshness | `current` with checked generation and read time; `stale` with reason; `unavailable` with failed source and reason |
| Pair gate | `awaiting_pair_completion`; `awaiting_scores` with missing checks; `continue_permitted` with sealed score hashes and rule decision; `stop_required` with sealed evidence and reason |
| Attempt trust | `awaiting_conditions`; `valid`; `invalid` with evidence and reason; `unknown` with missing evidence |
| Measurement | `measured` with value and unit; `not_collected`; `collection_failed`; `not_applicable` |
| Usage attribution | `awaiting_collection`; `execution_confirmed` with collection boundaries and source locations; `ambiguous_execution` with conflicting or missing evidence; `not_applicable` for a non-usage measure |
| Scorer labels | `concealed` with commitment and operator; `revealed` with matching mapping and all sealed score hashes; `not_applicable` for automated checks |
| Pair scores | `awaiting_scores`; `sealed_scores` with scorer receipt, rubric hash, output hashes, and decisions |
| Frequency coverage | `collecting` with fixed window and producer; `complete` with sealed source evidence; `incomplete` with missing dates or fields; `not_required` for a study with no volume claim |
| Analysis | `not_ready` with missing requirements; `computed` with effect, bounds, N, and rule result |
| Interval | `bounded` with lower and upper; `unbounded` with sample count and reason |
| Applicability | `current`; `needs_recheck`; `evidence_unavailable`; `withdrawn` with reason; `superseded` with successor ID |
| Adoption | `not_applied`; `applied` with activation evidence; `rolled_back` with restoration evidence |
| Follow-up | `scheduled` with trigger and scope; `check_due` with missing requirements; `checked` with scope and evidence |

Follow-up scope is `correctness_only` with named observable checks, or `paired_repeat` with a registered repeat ID. A `correctness_only` result cannot update the date or strength of the gain evidence.

A decision event supplies the reason, owner, evidence hash, and authorized scope. An adoption event supplies applied-change evidence. A supersession event supplies the replacement ID. These states cannot exist without their required data.

An attempt keeps its outcome and its trust assessment separately. For example, a failed task can still be a valid observation. An interrupted collection can leave a known task failure with unavailable timing.

### Smallest new tool set

Build one binary with one record model. Reuse existing launchers, build controls, store readers, and unit records. Add the missing protocol, execution receipt, conditions, score gate, analysis, and discovery functions to that binary. Hook adapters and the app read the same outputs. They do not compute a second verdict.

Sources for reuse: `~/.claude/scripts/buildlog/cli.py`, `~/.claude/scripts/usage/usage_report.py`, `~/.claude/scripts/history/history.py`, and `~/.claude/scripts/agents/mesh_runs.py`. Section 13 defines the interface.

The detached runner recognizes live mesh workers. It returns exit 3 for other callers. Select the caller's existing completion route before launch. Do not interpret that exit as a started experiment. Source: `~/.claude/scripts/agents/mesh_runs.py`.

## 13. Tooling

### CLI

Proposed workspace crate: `crates/cargo-experiment`. Both `cargo experiment` and `cargo-experiment` invoke it. SDLC owns the final name. The design includes no implementation.

| Command | Effect |
| --- | --- |
| `register <protocol-file>` | Refuse unsupported designs; validate and seal a protocol; append its registration; return the receipt |
| `run start <id> --pair <pair-id>` | Check conditions and prior gate; reserve both arm slots; launch one job that runs the pair in order |
| `run start <id> --slot <slot>` | Start one registered frequency-baseline observation; refuse a comparison-arm slot |
| `run record <id> --pair <pair-id> --evidence <file>` | Import the sealed pair result, arm receipts, and prevention evidence in one transaction |
| `run record <id> --attempt <attempt-id> --evidence <file>` | Import the durable workload receipt through its launch association; record conditions and measurements |
| `run record <id> --coverage <file>` | Import a prospective frequency manifest; record complete or missing coverage without inventing an attempt |
| `run score <id> --pair <pair-id> --evidence <file>` | Import sealed scores separately; apply concealed mapping if required; append the stop decision and gate |
| `analyze <id> --stage aa` | Compute N and the A/A check; permit A/B only when it passes |
| `analyze <id>` | Compute the final effect and rule result from the frozen data |
| `decide <id> accepted\|rejected\|not_proven --basis <file>` | Record the authorized decision; refuse unsupported acceptance |
| `adopt <id> --evidence <file>` | Record checked activation, archive, rollback, and follow-up |
| `find --subject <subject>` | Find prior records, including rejected and superseded work |
| `show <id>` | Show protocol, progress, evidence, decision, and adoption |
| `board [--owner <id>]` | Show open work and due follow-up checks |
| `check <id> --evidence <file>` | Record the scheduled adoption check and applicability |
| `supersede <id> --with <new-id> --basis <file>` | Link a later result without deleting the earlier record |

All commands support `--json`. Lookup commands read projections through the generation check in section 12. They do not collect stores, repair journals, acquire holds, or run experiments. A stale projection returns `stale` with exit 3. An unreadable or missing publication or index returns `unavailable` with exit 4 and a repair instruction. Valid JSON alone does not establish freshness. A separate `repair-index` command rebuilds only the derived index under the owner-store lock.

`run start` cannot accept an arbitrary replacement command. It resolves argument arrays and arm order from the sealed protocol. It records the input hash and conditions immediately before launch. Section 12 governs launch and recovery. A launcher receipt can establish `running`. Only durable workload receipts and the pair result establish pair completion. A completed pair still needs its score gate. Missing execution evidence gives `execution_unknown` and blocks relaunch.

Only one pair job or baseline job may be active for a serial protocol. Its pair adapter never runs arms concurrently. The next pair cannot start before `run score` records `continue_permitted` for the previous pair. A stop decision prevents every later launch. The next slot cannot skip a missing pair. Apply the section 6 failure table to preflight refusals and stopped pairs. The start command does not wait for load to improve. A retry follows a condition-change notice or an explicit new action, never a polling loop.

A protocol with a prospective frequency baseline cannot start A/A until its fixed window closes with complete coverage. `run record --coverage` seals daily manifests, including dates with no use. The protocol fixes the volume rule. Missing required coverage stops the study as `not_proven`. A protocol with an existing complete source, or an exercise with no volume claim, starts directly at A/A.

`analyze` pins the measurement, score, and frequency-coverage event sequences and their evidence hashes. New evidence requires a new analysis event. `decide` refuses an analysis whose inputs changed. Analysis and display events do not change those inputs. `adopt` requires an accepted decision and its permitted scope. A claimed role on a command line is not proof of authority. The basis file must cite the recorded unit, showrunner, or user instruction.

### JSON envelope

Use cargo-berth's stable front-end shell. Its required fields are `verb`, `status`, `exit_code`, `message`, `blocked_by`, and `presentation`. The presentation is `not_provided` or `rendered_blocks`. Each block has `summary` and `detail`. Source: `docs/cargo-berth/as-built/json-contract.md`.

Proposed example:

```json
{
  "output_contract_version": 1,
  "verb": "run start",
  "status": "conditions_refused",
  "exit_code": 2,
  "message": "Run not started: natedev load 19 exceeds protocol limit 8.",
  "blocked_by": [],
  "presentation": {
    "kind": "rendered_blocks",
    "blocks": [{
      "summary": "Run not started.",
      "detail": "natedev load 19 exceeds protocol limit 8."
    }]
  },
  "payload": {
    "kind": "conditions_refused",
    "data": {
      "attempt_id": "example-attempt",
      "condition": {"kind": "load_limit", "observed": 19, "maximum": 8}
    }
  }
}
```

Keep `blocked_by` for actual berth reservation IDs. Put load, machine, cache, and evidence reasons in tagged payload data. Do not put experiment IDs into berth's reservation field.

Use exit 0 for a completed operation, including a computed negative result. Use exit 2 for invalid input or refused conditions. Use exit 3 for incomplete evidence. Use exit 4 for unreadable records. The workload exit code stays in the attempt record.

Generic readers accept unfamiliar verbs and statuses. They use the stable shell for display. An unfamiliar presentation kind falls back to `message`. Specialized readers validate the payload version. Unsupported versions produce `unsupported_version`, never a positive decision.

Version journal events separately from display output. Preserve old event meanings. Append corrections with references to the original event. Do not rewrite history when a field name changes. The berth journal and display contract make this distinction. Source: `docs/cargo-berth/as-built/json-contract.md`.

### Hooks

Use two hook adapters: subject lookup and run-condition checks. Keep the CLI's start check authoritative. Hooks provide context and prevent accidental launches. They do not grant machine access.

| Event | Applies when | Output and action |
| --- | --- | --- |
| `SessionStart` on startup, resume, clear, or compact | Session subject matches a prior result | One context line with ID, evidence state, applicability, and report path |
| `PreToolUse` on `Edit`, `MultiEdit`, `Write`, or `apply_patch` | Edited paths match a result not yet shown in this context | One advisory line; the existing berth hook still controls edit access |
| `PreToolUse` on a recognized `run start` | Machine, hold, or load violates its protocol | Refuse that launch with one reason line |
| `PostToolUse` on a recognized experiment launch or record call | New evidence shows a condition breach or interrupted collection | One line naming the attempt and its trust state |

Print these example lines only when the checked record supports them:

```text
Experiment e42 is adopted for cargo-berth test fixtures on natedev. Report: <path>.
Run e42/ab-03 not started: protocol requires mac; command targets natedev.
Run e42/ab-03 is untrusted: load reached 19; protocol limit is 16.
```

Print nothing for no match, unchanged evidence, or a valid start. Never print a generic success line on every tool call. Key advisory delivery by session, context generation, experiment ID, and decision and applicability hashes. Compaction starts a new context generation. Key a condition notice by attempt and condition event. Repeated checks of that same event produce no second notice.

An accepted result that is not adopted is evidence, not an instruction to change behavior. Label rejected and unproven results as prior findings. Show their limits. Only an adopted result with current applicability and `current` lookup freshness can supply a current default. Key cached matches by the checked publication generation. An old valid index cannot satisfy this rule.

A repeated refused launch still returns the harness's required denial fields and status. It need not repeat an advisory context line. The CLI remains the source of the full reason on demand. Deduplication must never turn a refusal into permission.

Use Claude's `hookSpecificOutput.additionalContext` for context. For a denied Claude tool call, use the harness's permission-denial response. Codex's current `PreToolUse` path accepts a refusal and is silent on allow. Put Codex advisory context in its session or Work Order input through the launcher. Do not assume that Codex displays allowed-hook text. Sources: `~/.claude/scripts/hooks/session-start-delegate-resume.py`, `~/.claude/scripts/hooks/pre-tool-use-berth.py`, `~/.claude/settings.json`.

For process subjects, use explicit subject IDs from the Work Order or skill invocation. Do not infer a subject from arbitrary prose. For file edits, match canonical repository identity and component-based paths. A result's model, machine, and applicability conditions must also match.

The start check runs again after the hook. A pre-tool observation can become stale before launch. The completion adapter records changing conditions during detached work. `PostToolUse` alone cannot observe a long detached run.

If a required condition cannot be read, refuse the experiment start with `conditions_unknown`. If prior-result lookup fails, allow unrelated work and print one lookup-failure notice only when that session needs the index. Do not claim that no prior result exists.

Hook work reads local publication state, the checked index, and current condition files only. It makes no network request and scans no transcript. On the Mac, a mirror is a dated historical record and remains `stale` for current-default lookup. The launcher obtains an owner-checked result before a recorded experiment. Ordinary edits remain available without that lookup. Print one stale-source notice when relevant; do not present a mirror row as a current instruction.

Set a proposed 50 ms hook budget. Cache subject matches by index generation. Measure cold and warm hook wall time before enabling the hooks. Existing berth work found substantial cost from repeated Git operations. Do not add an equivalent repository scan to each edit. Source: `docs/cargo-berth/as-built/hook-cost.md`.

### cargo-handler view

Add an `Experiments` view beside the existing agent summary. Selecting a showrunner or unit filters the view to that owner. A top-level selection shows all records in the local projection. Do not create an agent tile for each experiment.

The app already groups managed agents under their managers. Reuse those identities to resolve the owner. Sources: `docs/cargo-handler/as-built/hierarchy.md`, `crates/cargo-handler/src/summary/mod.rs`.

Use simple text:

```text
Experiments   owner: experiments-unit   updated: 16:40
Subject          State       A/A       A/B        Noise       Verdict
history-command  running     8/8       7/12       0.004 s     not ready
test-fixture     accepted    8/8       12/12      0.03 s      gain 0.14 s
```

The detail view shows the hypothesis, primary measure, pairs done and left, invalid attempts, N, effect interval, and decision. It also shows the next action, report path, adoption state, and next-check date. Label examples as examples until real data exist.

Count pairs separately from attempts. A replacement does not increase completed pairs twice. Show `launch_pending`, `running`, `execution_unknown`, and `awaiting_scores` as distinct states. Do not display a projected completion count as evidence.

The CLI supplies the projection. The view does no analysis. Read it off the UI thread. Keep the first frame independent of the experiment source. The existing startup design already requires that behavior for census work. Source: `docs/cargo-handler/as-built/startup.md`.

Use `loading`, `current`, `stale`, and `unavailable` display states. Take freshness from the publication check, not JSON validity or file age. Retain the last successful view with its generation and timestamp after a read failure. Do not show stale progress as current. An idle view performs no builds, store collection, or network scan. The first version is read-only; decisions remain CLI operations with recorded evidence.

SDLC must confirm the binary name and the owner of the shared index. These are process assignments.

## Decisions from the user

- First live study (2026-10-25): the history-command trial in `worked-examples.md` §6, with and without `python3 -S`, runs first. It exercises the framework only. It claims no recurring savings and cannot authorize adoption. It starts after the user's design review is complete.
- Spending (2026-10-25): experiment work is normal work. Work the user asks for is approved. There is no spending limit (section 8).
- Evidence retention (2026-10-25): the raw evidence of a result that is not adopted stays for 90 days after closure (section 12).
