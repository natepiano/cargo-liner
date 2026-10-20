# Experiment framework

This is a design. It changes no tool or default. The proposed tool name is `cargo-experiment`.

An experiment tests one claim against a fixed control. Record the rules before the first run. Keep all attempts. Measure variation before the comparison. Check the result in normal work. Adopt a change only within the conditions that the evidence supports.

The framework uses the existing build log, usage ledger, and run history. One experiment record joins their evidence. The record also holds the decision, the adoption, and the next check.

## One-page procedure

1. Find prior results for the repository, paths, and process. Read each applicable result and its current state.
2. Name the owner, question, control A, and change B. State one falsifiable claim.
3. Choose one primary measure. Define its unit, start, end, and minimum useful gain.
4. Define correctness checks. Define each permitted secondary cost and its limit.
5. Select the tasks and conditions before you see results. Set the A/A count, A/B count, run order, deadline, and budget.
6. Register the protocol. Seal its input files and analysis rules with hashes. Record the authority to run it.
7. Arrange the worktree reservation and machine access. Use the existing unit and build controls.
8. Run A/A pairs. Compute the observed noise floor. Stop if the conditions cannot support the stated claim.
9. Run the registered A/B pairs. Keep failed, interrupted, and invalid attempts with their reasons.
10. Apply the registered analysis once. Report the effect, interval, noise floor, coverage, and correctness results.
11. Check the effect in the registered live workload. Estimate daily or weekly gain from observed use.
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

Register one immutable `protocol.json` before any measured run. Section 12 defines its location and fields. The registration event stores its SHA-256, UTC time, owner, and authorization reference.

The protocol must fix these rules:

- Question, claim, A, B, and primary measure.
- Task eligibility and selection order.
- Independent sample unit and any groups of related samples.
- A/A count and A/B count.
- Run order or the exact seeded allocation algorithm.
- Cache preparation and excluded preparation work.
- Conditions that prevent a start or invalidate an attempt.
- Minimum gain, analysis, confidence level, and correctness limits.
- Replacement limit, time limit, resource limit, and stop rule.
- Live check, adoption scope, rollback, and follow-up rule.

A future input can have a fixed selection rule. For example, select the next 12 eligible review tasks. Seal each task before either arm sees it. Do not select tasks from their observed results.

Use `awaiting_input_selection` for inputs that the selection rule has not selected. Use `sealed_inputs` after selection. A run cannot start with required inputs still unsealed.

Keep registration, raw attempts, and decisions as separate records. Each attempt names the protocol hash. The journal gives events increasing sequence numbers. An event also contains the previous event's hash. A registration receipt goes in the owning unit's phase record before its first launch. This supplies a separate reference to the protocol hash. A local hash alone cannot prove when an author wrote a file.

Do not edit a registered rule. A necessary change creates a new protocol revision and a new registration receipt. Close the old comparison as `not_proven` if its required data are incomplete. Do not combine results across revisions for acceptance. Label any later analysis of the old data `exploratory`.

The normal stop is the fixed sample count. Stop earlier for a correctness failure, a resource limit, or the deadline. Do not stop early because the effect looks good. Do not add runs because an interval almost meets the rule.

## 3. Baseline and control

### Code

Pin the common base commit. A contains that base. B contains that base plus the exact change. Record the full commit IDs and any patch hash. A moving branch name is insufficient.

Use separate, owned worktrees or scratch clones. Give them equivalent paths, dependency versions, toolchains, and target state. If path-dependent cache keys matter, include path as a controlled factor. Do not use another unit's working tree.

Prepare each arm by the same rule before each pair. Run adjacent pairs in balanced order: A/B, then B/A. A registered seeded order is also valid. Do not run all A samples before all B samples.

A pair uses the same input and machine. Treat natedev and the Mac as separate populations. Report their results separately. A natedev result does not establish a Mac result.

### Process and model work

A task pair uses the same frozen input, acceptance requirements, tools, and time limit. Each arm receives an independent context. Do not show A's answer or review to B.

Pin the prompt and skill text by content hash. Pin the model ID, effort, service tier, account label, and launcher version. Record context reuse, cache age, and compaction state. A model alias does not promise identical future model behavior.

For a review comparison, score both outputs with the same rubric. Hide arm labels from the scorer. Count distinct confirmed defects, missed required defects, and incorrect findings. A longer list of findings is not evidence of better review.

For real phases that cannot run twice, allocate comparable tasks before work starts. Block the allocation by task class and unit. Keep all assigned tasks in the outcome report. Record abandoned tasks as outcomes. Use a fixed observation window for later defects and user corrections.

Repeated requests within one session are not independent samples. The task or phase is the sample. If one unit or day creates common variation, analyze that group as one sample. Set the number of groups before the run.

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

Invalidate a controlled pair when an unrelated workload breaks the limit, a required measurement is missing, or its machine changes. Keep both arms and the reason. Replace the whole pair in the next registered slot. Report invalid counts by arm and cause. Unequal losses can make a result `not_proven`.

An arm failure is not an invalid environment sample. Preserve its elapsed time and failure outcome. A missing result is not zero. Use `not_collected` or `collection_failed`, with a reason.

Define cold and warm precisely. For example, `empty_private_target` does not mean an empty sccache. Use private cache state for controlled cold runs. Never clear a shared cache to prepare an experiment. For warm runs, define the warmup command and count. Keep warmup cost in the cost record.

`verify.sh` can return stored results without executing the workload. Its cache key includes tree and tool inputs. Record reuse as reuse. Do not count it as another execution sample. Sources: `~/.claude/scripts/direct/verify.sh`, `~/.claude/scripts/buildlog/record.py`.

The build log's machine samples record memory, pressure, and power state. They do not supply one-minute load. The proposed run record must collect that missing field. Sources: `~/.claude/scripts/buildlog/sample.py`, `~/.claude/scripts/buildlog/index.py`.

## 5. Measures and existing evidence

Wall clock is the only time measure used to claim a time gain. CPU time, process counts, and summed task durations can explain a result. They cannot replace elapsed time. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/reduce-test-load.md`.

| Experiment | Usual primary measure | Secondary measures and correctness evidence |
| --- | --- | --- |
| Build or lint change | Complete invocation wall seconds | Failures, diagnostics, rebuilds, process starts, cache reuse |
| Test change | Full relevant suite wall seconds | One-test wall seconds, coverage, failures, retries, CI result |
| Model, prompt, or skill | Wall seconds per completed task, or weighted tokens per assigned task | Confirmed defects, missed requirements, cost, corrections, repair rounds |
| Review regime | Defects that remain after review per assigned phase | Start-to-merge wall time, reviewer cost, repairs, user corrections |
| Application change | Wall latency for a named user action | Correct output, completion rate, resource use |

Choose one primary measure from the appropriate row. Define quality and cost limits separately. Count a timeout or abandoned task in the quality result. Do not report only successful tasks.

For an end-to-end time claim, start at the request and end at the usable result. Include queues, copies, and necessary repairs. A controlled execution-only measure may exclude queue time, but its name must say so. Record queue time separately. Use a monotonic clock for elapsed time. Use UTC for event joins.

| Source | What it supplies | Query route and limits |
| --- | --- | --- |
| `~/.claude/scripts/buildlog/store.py`, `index.py`, `cli.py` | Host records, calls, steps, tests, CI, and a derived index | `buildlog schema`; `buildlog query --json '<SQL>'`; preserve SQL and window |
| `~/.claude/commands/build/report.md`, `mac_report.md` | Human build reports and Mac outcome coverage | Reports are on demand; keep host coverage separate |
| `~/.claude/docs/as-built/usage-ledger.md`; `scripts/usage/usage_report.py` under `~/.claude/` | Requests, token classes, roles, phases, passes, findings, and source states | `usage_report.py list`; named metrics such as `weighted` and `wait_share` |
| `~/.claude/scripts/usage/weights.py` | Dated weights by model | Save the weight version with the result |
| `~/.claude/scripts/history/history.py` | Stage, phase, and run elapsed times | `phases --status all --format json`; set an explicit time window |
| `~/.claude/scripts/production/review_regime.py` | Phase review outcomes | `report --since <date>`; preserve ledger rows and their field meanings |
| `~/.claude/scripts/model_study/model_study.py` | Director request comparison and phase analysis | Read saved results; its commands also write derived files |
| `~/.claude/scripts/nightly_review/candidates.py`; `~/.claude/commands/efficiency_next.md` | Measured proposals and adoption outcomes | Link the experiment from the existing proposal line |

The build log stores permanent JSON lines under `~/.local/state/buildlog/<host>/`. Its `index.sqlite` is derived. `buildlog query` updates that index before its read-only SQL query. For a strictly read-only investigation, open the existing index with SQLite `mode=ro`. Record its freshness. Sources: `~/.claude/scripts/buildlog/store.py`, `cli.py`.

The usage ledger is `~/.local/state/usage-model/usage.db` on natedev. Its report opens a read-only snapshot. Save source coverage with each query. A missing Mac read is not evidence of zero Mac use. Sources: `~/.claude/docs/as-built/usage-ledger.md`, `~/.claude/scripts/usage/usage_report.py`.

Weighted tokens, API-equivalent dollars, and subscription use are different measures. Do not convert between them without a stated model. Unpriced usage has state `unpriced`. Do not assign it zero cost. The director study's price table and the usage ledger's weights serve different purposes. Sources: `~/.claude/scripts/model_study/stats.py`, `~/.claude/scripts/usage/weights.py`.

No inspected general store supplies a complete process-start count or a verified user-correction count. Collect these in the experiment when required. Name the trace or correction event. Deduplicate defects by cause and affected behavior. Keep scorer decisions with source references.

Review-pass count and repair-round count differ. The token study found unusable `fix_pass` values for its review analysis. Read the phase events for the required count. Sources: `~/.claude/docs/as-built/token-efficiency-study.md`, `~/.claude/scripts/history/history.py`.

## 6. Repeat runs and noise floor

### One default analysis

Use paired differences. For a lower-is-better measure, define `d = A - B`. Positive d means an improvement. Reverse the sign for a higher-is-better measure.

Use the median of independent pair differences as the primary effect. Do not subtract two unpaired medians and call it a paired result. Show every pair beside the summary.

The default protocol has eight A/A pairs and twelve A/B pairs. These are starting budgets, not a promise of sufficient evidence. Choose larger fixed counts before registration when the effect is small or the risk is high. State the budget and precision reason.

For expensive agent work, use whole tasks as pairs. Six to eleven independent pairs can support only large, consistent effects. Fewer than six pairs give no finite two-sided 95% median interval under this rule. Report them as exploratory and `not_proven`.

### A/A stage

Run the same baseline through both preparation paths. Apply the planned order, caches, tasks, and collectors. A/A work must exercise the variation expected in A/B work.

Let `a_j = A_first - A_second`. The observed noise floor N is the nearest-rank 90th percentile of `abs(a_j)`. With eight pairs, N is their maximum. Report the signed median and range as well. This is an observed limit, not a population guarantee.

Stop if the A/A median interval excludes zero. The preparation paths or time trend need investigation. Stop if N exceeds M. A more precise study needs a new protocol, not a larger sample chosen after B appears.

Keep A/A costs in the experiment budget. Do not include A/A observations in the A/B effect.

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

### Failures and replacements

Register at most two replacement pairs by default. An environment-invalid pair does not consume the valid-pair target. It consumes the attempt and resource budgets. Keep it in the result table.

A correctness failure stops the experiment and rejects the change within that scope. A deadline, exhausted budget, incomplete pair, or unavailable required source gives `not_proven`. Do not replace a treatment failure with a successful retry.

## 7. Live effect and daily value

A controlled result establishes an effect only for its measured conditions. It does not establish a useful gain in production.

Register a live check before the controlled comparison. Define the task mix from a fixed store window. Include failures, fixes, cache states, repeated calls, and inactive callers. Keep task selection independent of the arm's result.

Prefer randomized or interleaved use within one reversible unit. If real use cannot be allocated, report the live comparison as observational. State what remains unproven. A default change may remain a limited pilot until stronger evidence exists.

Estimate weekly total savings by workload class:

`weekly saving = sum(observed calls per week * mean saving per call) - added weekly operating cost`.

Keep setup, installation, maintenance, and rollback costs in a separate table. State the payback period if that informs the decision. Do not assign a recurring gain to a caller with no observed use.

Freeze the frequency window before analysis. Save the SQL, source rows, host coverage, and time zone. Use low and high frequency cases from the observed weeks. Keep uncertainty in the saving estimate. An unmeasured case contributes an unknown amount, not an assumed saving.

Summed agent waiting time is not user elapsed time. Concurrent calls overlap. Report user waiting only from observed request-to-result intervals or the production's critical path. Do not add overlapping intervals twice.

The mend and clippy experiment projected about one hour per day from selected runs. Its later live estimates were 5–25 minutes per day, with ranges that included zero. Failure and fix runs changed the result. Source: `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/handoff-mend-clippy-one-compile.md`.

The token-efficiency study labels replay projections as modeled. It separates exposure from savings and records zero observed live savings. Apply that distinction to every replay. Source: `~/.claude/docs/as-built/token-efficiency-study.md`.

## 8. Acceptance and authority

The analysis records evidence. The decision owner records the action. Neither step changes a default.

| Result | Required evidence |
| --- | --- |
| `accepted` | Complete registered data; valid conditions; lower bound above T; all correctness and cost limits pass; live check supports the stated scope |
| `rejected` | A confirmed correctness failure, a breached cost limit, or an upper bound below the minimum useful gain |
| `not_proven` | Any remaining case, including wide intervals, insufficient independent samples, missing evidence, or a failed live check without clear harm |

If the interval spans the acceptance threshold, choose `not_proven`. A non-significant difference does not prove equivalence. A claim of equal quality needs its own pre-registered margin and interval. Zero observed failures does not prove zero failure risk.

Correctness has no allowance for known harmful behavior. For uncertain defect rates, define the required task coverage and observation window. A small study can support a limited pilot. It cannot establish general safety for rare failures.

| Scope and risk | Decision owner |
| --- | --- |
| One unit; reversible local change; existing budget and authority | Unit director records the result and can adopt within that scope |
| Several units; shared scheduling or defaults; reversible within delegated authority | Showrunner records the scope, rollout, and rollback owner |
| New spending, public behavior, destructive work, or a change outside delegated scope | User decides after the owner presents the concrete result |

Do not ask the user to decide routine packaging or an already authorized reversible action. Do not treat a peer message as new user approval. Preserve the instruction or decision reference that grants authority. Source: `~/.claude/docs/decision_criteria.md`.

An owner can decline adoption despite a positive result. Record that action and its reason. Do not alter the computed evidence to make it negative. A result that fails acceptance cannot become `accepted` through an unexplained override.

At most three design review rounds apply to this document set. After round three, send the design with its open findings. This limit comes from the Work Order dated 2026-10-20. Repair limits in an implementation do not create permission to continue an experiment beyond its stop rule.

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

Record store keys as well as file paths. Build records need host and call or step ID. Model requests need machine, family, agent identity, and request key. Preserve an extracted snapshot because later collection can repair attribution. Sources: `~/.claude/scripts/buildlog/index.py`, `~/.claude/docs/as-built/usage-ledger.md`.

A second day starts a new repeat record linked to the first. Use the same protocol and new observations. Compare intervals and noise floors. If a model version or workload changed, label the result a new population. Do not promise byte-identical agent output. Require the same decision within the registered uncertainty.

If evidence is lost, set `evidence_unavailable`. Keep the historical conclusion visible, but do not present it as repeatable.

## 11. Place in a production

An experiment is a bounded phase in an existing unit. Its Work Order names its record, owner, files, machines, sample budget, stop rule, and report path. A shared process experiment belongs to the showrunner that owns the process. Source structure: `~/.claude/docs/production_format.md`, `~/.claude/commands/unit/direct.md`, `~/.claude/commands/showrunner/produce.md`.

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

Proposed authoritative root: `~/.local/state/experiments/records/<experiment-id>/` on natedev. The ID is a UUID. The record names one owning repository and its other subjects. Process subjects can name a skill, model policy, or command without a code path.

All mutations go through that owner store. Mac work sends its completion evidence to the natedev launcher. An unavailable owner prevents new recorded runs. It does not permit a second writable copy. This design adds no cross-machine merge protocol.

| File | Owner and content |
| --- | --- |
| `protocol.json` | Immutable registered rules and input selection method |
| `events.ndjson` | Append-only attempts, analysis, decisions, and adoption events |
| `evidence/` | Immutable content-addressed inputs, outputs, and source extracts |
| `result.md` | Generated report with evidence links and applicability |

Use one short exclusive file lock for each journal append. Never hold it while a job runs. An event has `schema_version`, `event_id`, `sequence`, `at_utc`, `protocol_sha256`, `previous_event`, and a tagged `event`. `previous_event` is `journal_start` or `prior_event` with its hash. Completion events have an idempotency key from the launcher and attempt ID. A repeated identical completion changes nothing. A conflicting completion is refused.

The root also holds a derived `index.json`. It maps subject keys to record IDs, current states, and report paths. Writes replace it atomically. It is not an evidence source. Rebuild it from the journals if necessary. Do not add a database until measured query cost requires one.

Back up the authoritative root with the owner's durable data. Before adoption, archive the sealed protocol, final report, and evidence manifest in the owning repository's `docs/experiments/results/<id>/`. This is a fixed export, not a second writable record. Its hashes must match the owner record. Keep raw evidence for the life of an adopted result. Record a retention decision before removing any other evidence.

### Protocol fields

| Field | Required content |
| --- | --- |
| `identity` | ID, title, owner, repository identity, path and process subjects |
| `authority` | Scope, budget, and source of authorization |
| `claim` | Question, mechanism, population, A, B, primary measure, M |
| `inputs` | Sealed inputs or a fixed future selection rule |
| `execution` | Argument arrays, working directory rules, preparation, collection, and completion adapter |
| `conditions` | Machine rules, load rules, holds, caches, model, account, and tools |
| `sampling` | Pair unit, groups, counts, order, replacement limit, deadline, and resource budgets |
| `analysis` | Direction, formula, interval rule, noise rule, and limits |
| `correctness` | Required checks, scorer rubric, observation window, and stop conditions |
| `live_check` | Workload window, allocation, measures, and required scope |
| `adoption_plan` | Target, decision owner, rollback, and follow-up rule |

### Attempt fields

Each measured arm has one attempt record. A pair joins two attempt IDs. A replacement pair gets new IDs and references the invalid pair.

| Field | Required content |
| --- | --- |
| `identity` | Experiment ID, protocol hash, slot, attempt ID, pair ID, stage, and arm |
| `work_item` | Input hash, eligibility evidence, task or group ID, and source commit |
| `execution` | Sealed command ID, actual arguments, directory, host, and launcher run ID |
| `conditions` | Before, during, and after observations with timestamps and source states |
| `outcome` | Named attempt state; workload exit, timeout, or interruption evidence |
| `trust` | Named trust state and every condition violation |
| `measurements` | Primary value, secondary values, units, and collection states |
| `evidence` | Content hashes, source row keys, output paths, and scorer record |

A slot names one arm, such as `ab-03-B`. The protocol fixes its pair and order. The board counts a pair only after both required arms have valid records.

### Named states

Use tagged records. Do not use an empty string, absent field, or `null` for a domain state.

| Record | States and required data |
| --- | --- |
| Experiment lifecycle | `registered`; `running`; `accepted`; `rejected`; `not_proven`; `adopted`; `superseded` |
| Running experiment | Stage `aa`, `ab`, or `live`; progress `ready`, `active`, or `paused` with a reason |
| Attempt | `prepared`; `not_started` with reason; `running`; `completed`; `failed`; `interrupted` |
| Attempt trust | `awaiting_conditions`; `valid`; `invalid` with evidence and reason; `unknown` with missing evidence |
| Measurement | `measured` with value and unit; `not_collected`; `collection_failed`; `not_applicable` |
| Analysis | `not_ready` with missing requirements; `computed` with effect, bounds, N, and rule result |
| Interval | `bounded` with lower and upper; `unbounded` with sample count and reason |
| Applicability | `current`; `needs_recheck`; `evidence_unavailable`; `withdrawn` with reason; `superseded` with successor ID |
| Adoption | `not_applied`; `applied` with activation evidence; `rolled_back` with restoration evidence |
| Follow-up | `scheduled` with trigger; `check_due`; `checked` with evidence |

A decision event supplies the reason, owner, evidence hash, and authorized scope. An adoption event supplies applied-change evidence. A supersession event supplies the replacement ID. These states cannot exist without their required data.

An attempt keeps its outcome and its trust assessment separately. For example, a failed task can still be a valid observation. An interrupted collection can leave a known task failure with unavailable timing.

### Smallest new tool set

Build one binary with one record model. Reuse existing launchers, build controls, store readers, and unit records. Add the missing protocol, conditions, analysis, and discovery functions to that binary. Hook adapters and the app read the same outputs. They do not compute a second verdict.

Sources for reuse: `~/.claude/scripts/buildlog/cli.py`, `~/.claude/scripts/usage/usage_report.py`, `~/.claude/scripts/history/history.py`, and `~/.claude/scripts/agents/mesh_runs.py`. Section 13 defines the interface.

The detached runner recognizes live mesh workers. It returns exit 3 for other callers. Select the caller's existing completion route before launch. Do not interpret that exit as a started experiment. Source: `~/.claude/scripts/agents/mesh_runs.py`.

## 13. Tooling

### CLI

Proposed workspace crate: `crates/cargo-experiment`. Both `cargo experiment` and `cargo-experiment` invoke it. SDLC owns the final name. No implementation is part of this phase.

| Command | Effect |
| --- | --- |
| `register <protocol-file>` | Validate and seal a protocol; append its registration; return the receipt |
| `run start <id> --slot <slot>` | Check inputs and current conditions; launch the registered command through the existing detached runner |
| `run record <id> --attempt <attempt-id> --evidence <file>` | Record a launcher completion, conditions, measurements, and raw evidence |
| `analyze <id> --stage aa` | Compute N and the A/A check; permit A/B only when it passes |
| `analyze <id>` | Compute the final effect and rule result from the frozen data |
| `decide <id> accepted\|rejected\|not_proven --basis <file>` | Record the authorized decision; refuse unsupported acceptance |
| `adopt <id> --evidence <file>` | Record checked activation, archive, rollback, and follow-up |
| `find --subject <subject>` | Find prior records, including rejected and superseded work |
| `show <id>` | Show protocol, progress, evidence, decision, and adoption |
| `board [--owner <id>]` | Show open work and due follow-up checks |
| `check <id> --evidence <file>` | Record the scheduled adoption check and applicability |
| `supersede <id> --with <new-id> --basis <file>` | Link a later result without deleting the earlier record |

All commands support `--json`. Lookup commands read existing projections. They do not collect stores, repair journals, acquire holds, or run experiments. A missing index returns `index_unavailable` with a repair instruction. A separate `repair-index` command rebuilds only the derived index.

`run start` cannot accept an arbitrary replacement command. It resolves an argument array from the sealed protocol. It records the input hash and conditions immediately before launch. A registered asynchronous job remains `running` until the completion adapter records the actual exit. The launcher's successful exit is not the workload's result.

Only one attempt may be active for a serial protocol. The next slot cannot skip a missing pair. A failed preflight creates `not_started` with a reason. It launches no work and consumes no replacement pair. It still consumes elapsed deadline time. The start command does not wait for load to improve. A retry follows a condition-change notice or an explicit new action, never a polling loop.

`analyze` pins the measurement-event sequence and evidence hashes. New measurements require a new analysis event. `decide` refuses an analysis whose measurement inputs changed. Analysis and display events do not change those inputs. `adopt` requires an accepted decision and its permitted scope. A claimed role on a command line is not proof of authority. The basis file must cite the recorded unit, showrunner, or user instruction.

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

Examples of true lines:

```text
Experiment e42 applies to process/review-pool on natedev: retain instructions after reset. Report: <path>.
Run e42/ab-03 not started: protocol requires mac; command targets natedev.
Run e42/ab-03 is untrusted: load reached 19; protocol limit is 16.
```

Print nothing for no match, unchanged evidence, or a valid start. Never print a generic success line on every tool call. Key advisory delivery by session, context generation, experiment ID, and decision hash. Compaction starts a new context generation. Key a condition notice by attempt and condition event. Repeated checks of that same event produce no second notice.

An accepted result that is not adopted is evidence, not an instruction to change behavior. Label rejected and unproven results as prior findings. Show their limits. Only an adopted result with current applicability can supply a current default.

A repeated refused launch still returns the harness's required denial fields and status. It need not repeat an advisory context line. The CLI remains the source of the full reason on demand. Deduplication must never turn a refusal into permission.

Use Claude's `hookSpecificOutput.additionalContext` for context. For a denied Claude tool call, use the harness's permission-denial response. Codex's current `PreToolUse` path accepts a refusal and is silent on allow. Put Codex advisory context in its session or Work Order input through the launcher. Do not assume that Codex displays allowed-hook text. Sources: `~/.claude/scripts/hooks/session-start-delegate-resume.py`, `~/.claude/scripts/hooks/pre-tool-use-berth.py`, `~/.claude/settings.json`.

For process subjects, use explicit subject IDs from the Work Order or skill invocation. Do not infer a subject from arbitrary prose. For file edits, match canonical repository identity and component-based paths. A result's model, machine, and applicability conditions must also match.

The start check runs again after the hook. A pre-tool observation can become stale before launch. The completion adapter records changing conditions during detached work. `PostToolUse` alone cannot observe a long detached run.

If a required condition cannot be read, refuse the experiment start with `conditions_unknown`. If prior-result lookup fails, allow unrelated work and print one lookup-failure notice only when that session needs the index. Do not claim that no prior result exists.

Hook work reads the small local index and current condition files only. It makes no network request and scans no transcript. A Mac mirror has a timestamp and record hashes. Stale mirrors cannot assert current acceptance. The launcher refreshes a mirror before a recorded experiment; ordinary edits remain available when it cannot.

Set a proposed 50 ms hook budget. Cache subject matches by index generation. Measure cold and warm hook wall time before enabling the hooks. Existing berth work found substantial cost from repeated Git operations. Do not add an equivalent repository scan to each edit. Source: `docs/cargo-berth/as-built/hook-cost.md`.

### cargo-handler view

Add an `Experiments` view beside the existing agent summary. Selecting a showrunner or unit filters the view to that owner. A top-level selection shows all records in the local projection. Do not create an agent tile for each experiment.

The app already groups managed agents under their managers. Reuse those identities to resolve the owner. Sources: `docs/cargo-handler/as-built/hierarchy.md`, `crates/cargo-handler/src/summary/mod.rs`.

Use simple text:

```text
Experiments   owner: experiments-unit   updated: 16:40
Subject        State       A/A       A/B        Noise       Verdict
review-pool    running     8/8       7/12       420 tokens  not ready
test-fixture   accepted    8/8       12/12      0.03 s      gain 0.14 s
```

The detail view shows the hypothesis, primary measure, pairs done and left, invalid attempts, N, effect interval, and decision. It also shows the next action, report path, adoption state, and next-check date. Label examples as examples until real data exist.

Count pairs separately from attempts. A replacement does not increase completed pairs twice. An active run has state `running`. Do not display a projected completion count as evidence.

The CLI supplies the projection. The view does no analysis. Read it off the UI thread. Keep the first frame independent of the experiment source. The existing startup design already requires that behavior for census work. Source: `docs/cargo-handler/as-built/startup.md`.

Use `loading`, `current`, `stale`, and `unavailable` display states. Retain the last successful view with its timestamp after a read failure. Do not show stale progress as current. An idle view performs no builds, store collection, or network scan. The first version is read-only; decisions remain CLI operations with recorded evidence.

SDLC must confirm the binary name and the owner of the shared index. These are process assignments. They do not block review of this design.

## Open questions for the user

1. Which first live study should run after approval? `worked-examples.md` proposes a limited review-context study.
2. What recurring spending limit may a showrunner approve without a separate user decision?
3. How long should non-adopted raw evidence remain after closure? This draft retains it until an explicit retention decision.
