# Experiment framework design

## What it is

This set of documents holds a design for an experiment framework. Agents use it to run experiments on any code base and on the work process. The goal is results that are reliable and repeatable, so that a good result can safely become a default. The design follows the scientific method. It goes from question and hypothesis, through pre-registration, a control, noise measurement and a paired comparison, to a recorded decision, adoption and a later check. It fits natedev and the Mac: build holds, load, `verify.sh`, berth, units and showrunners, and the test-time rules. The design is a proposal. Nothing is built. `cargo-experiment` is only a proposed name. Building it is new work that the user has not asked for yet.

## How it is organized

- [`../framework.md`](../framework.md) is the design. It has a summary, a one-page procedure, sections 1-13, the user's decisions, and the open question for the user.
- [`../worked-examples.md`](../worked-examples.md) applies the design to five past studies. It also gives one complete protocol for a first live study.

framework.md sections: 1. Question and hypothesis. 2. Pre-registration. 3. Baseline and control. 4. Conditions. 5. Measures and existing evidence. 6. Repeat runs and noise floor. 7. Live effect and daily value. 8. Acceptance and decision owner. 9. Adoption and keeping the result. 10. Repeatability. 11. Place in a production. 12. Records and tools. 13. Tooling. Then "Decisions from the user" and "Open questions for the user".

worked-examples.md sections: 1. Mend and clippy in one compile. 2. Review regime trial. 3. Cargo-berth test-time reduction. 4. Director model study. 5. Review-pool reset and fork probes. 6. First live protocol: a history-command framework exercise.

Key design elements:

- One supported study design: independent same-input pairs with a positive gain. `register` refuses other designs as `unsupported_design` (§3, §6 "One supported analysis").
- A/A noise floor N, exact order-statistic median interval, and threshold `T = max(M, N)` (§6).
- One failure table: preflight refusal, measured environmental breach, missing measurement, missing receipt and workload failure (§6 "Failures and replacements").
- One launcher job per pair. The arms run in order, each with its own receipt. The adapter seals a durable workload receipt before the runner can send its notice (§12 "One experiment directory").
- Mac timing and conditions come from the Mac. The adapter reuses the `mac_run.sh` claim and status mechanisms and copies the receipt back once (§12 "Mac execution and receipt transfer").
- One shared index, published under one owner-store lock and tied to journal generations. Lookup answers `current`, `stale` or `unavailable` (§12 "Index publication and freshness").
- Phase elapsed time comes from `phase_started` and `phase_finished` events joined by `phase_instance_id`, not from summed stage seconds (§5).
- Protocol fields, attempt fields and named states (§12).
- Proposed tools: one binary, CLI, JSON envelope, two hook adapters and a cargo-handler view (§12 "Smallest new tool set", §13).
- The user's decisions and one open question (end of framework.md).

## Invariants

- The set is a design only. The §12 states and the §13 tools are a proposal, not a contract.
- Correctness comes first, then simple, fast, beautiful. A faster result that is not correct fails.
- Register `protocol.json` before any measured run. Do not edit a registered rule. A change makes a new revision.
- Keep every attempt. A missing result is not zero.
- Wall clock is the only time measure for a time-gain claim.
- Only a measured, registered environmental breach permits a replacement pair. The default limit is two replacements.
- natedev and the Mac are separate populations. Mac measurements stay supported.
- One owner store on natedev holds all records. There is no second writable copy, no database and no cross-machine merge.
- Freshness comes from the publication check only. A timestamp or valid JSON does not prove it.
- The CLI start check is the authority. Hooks and the app read the same outputs and do not compute a second verdict.
- Domain states are tagged records. Do not use `null`, an empty string or an absent field for a state.
- Missing execution evidence gives `execution_unknown` and blocks later starts. Recovery imports the original evidence and never runs the work again.
- A `framework_exercise` cannot be accepted or adopted. `decide accepted` and `adopt` refuse it.
- SDLC owns the binary name and the owner of the shared index.

## Gotchas

- The existing stores have limits. The build log has no one-minute load. No store counts process starts or user corrections. A measure that needs one of these has no source until a build adds it (§4, §5).
- The last design fixes had no further review, because the design review had a limit of three rounds. They are the failure table, index freshness, the Mac receipt path, one job per pair, and the purpose of the first study. Review these first.
- `mesh_runs.py` sends its completion notice before it saves its finished record. Do not use its finished state as the workload result (§12).
- `mesh_runs.py` makes a new ID on each start, so there is no automatic retry. It returns exit 3 to callers that are not mesh workers. This exit does not mean that an experiment started (§12).
- `buildlog/sample.py` is Linux only. A Mac collector must read `sysctl`, `vm_stat` and `pmset` (§4, §12).
- SSH can report zero for a failed remote command. `mac_run.sh` reads an explicit `mac_exit=` line. The script is specific to Hana. Reuse its mechanisms, not its workload or its file names (§12).
- `verify.sh` can return stored results. Record reuse as reuse, not as a sample (§4).
- `history.py phases` `total_seconds` is a sum of stage time. It is not phase elapsed time (§5).
- `buildlog query` updates the index before it reads. For a read-only investigation, open SQLite with `mode=ro` (§5).
- A Codex ledger key is a source location, not a request ID. Codex usage cannot be a primary measure in the first version (§5).
- The load limits (8 to start, 16 untrusted, on 32 CPUs) are proposed experiment limits. They are not `verify.sh` rules (§4).
- The Codex `PreToolUse` hook is silent on allow. Send Codex context through the launcher (§13).
- The proposed hook budget is 50 ms. Do not add a repository scan to each edit (§13).
- In `review_regime.py`, the code-finding field changed meaning on 2026-10-07. Do not combine the two meanings (worked-examples §2).
- The worked-examples §6 protocol refuses registration until the pair adapter and the score importer exist. Running it also needs a request that starts execution and names the operator.

## Why

- Same-input pairs only: agent studies give few tasks. Unpaired allocation, equal-quality claims and retained-thread studies need their own designs for estimators and dependence. The exact interval does not give the narrow intervals that a bootstrap gives on a few tasks.
- One job per pair: the adapter measures the gap between arms itself. No dispatch or receipt import happens between the arms.
- Receipt before notice: this removes the race with the order in which `mesh_runs` sends its notice and saves its record. A lost notice then needs no rerun.
- One lock and generations: an index from before an interrupted update must answer `stale`, even if its JSON is valid.
- Wall clock and a live check: in the mend and clippy work, selected benchmark runs projected one hour per day. The live gain was 5-25 minutes per day.
- Ruled out, a reset-reviewer first study: units keep their reviewer and never reset it, so its control arm is not current policy.
- Ruled out, marking Mac measurements unsupported: the design must fit natedev and the Mac.
- The first live study (worked-examples §6), chosen by the user on 2026-10-25, only tests the framework. It compares the history command with and without `python3 -S`. No recurring workload is known for it. Its 0.010 s threshold saves only about 0.070 s per week. Its purpose is fixed as `framework_exercise`, with `live_check` `not_claimed` and adoption `not_permitted`. It therefore closes as `not_proven` for production gain, or `rejected` if B fails. It cannot authorize adoption.

## Open questions

1. How long must raw evidence that was not adopted stay after closure? The design keeps it until an explicit retention decision.
