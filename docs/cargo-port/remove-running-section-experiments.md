# experiments

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** An Astra designer writes a design for an experiment framework under `docs/experiments/`. An Astra reviewer reviews it for up to three rounds. The design then goes to the showrunner and the user. The output is a design only, with no code. A build of the design is new work that the user approves after they read it.

> **Production: remove-running-section** — unit `experiments-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-20 16:35 EDT

I want you to create a new unit director in charge of creating a design, working with Astra on the design and review process, as I have been discussing with all showrunners. But use that Astra design and review process with this new unit director to design an experimentation framework that can work for our agents to be able to run experiments on any of our code bases and also our process, in order to be able to have reliable and repeatable results with respect to improvement. Like, if we run an experiment and we find a good result, then we're going to want to use this. Let's use the scientific method, and it needs to design that for our environment.

## Delegation Context

- **Project:** the experiment framework design. It is a document set under `docs/experiments/`. It changes no crate and no script.
- **Project started:** 2026-10-20T16:37:00-04:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-experiments`, branch `remove-running-section-experiments`. The designer works only here. Every Work Order names the worktree and branch that git reports at dispatch.
- **Base commit:** `5e0ba048`.
- **Designer:** one Codex thread, model `gpt-6-astra`, reasoning `xhigh`. It writes the first draft and makes each revision in the same thread. The unit director resumes the thread by hand. The thread id and its `CODEX_HOME` go in the unit notes, so that a later designer launcher can adopt the session (natedev, 2026-10-20). The user allows one designer session and one reviewer session.
- **Reviewer:** `/astra_review <document> [focus]` (`~/.claude/commands/astra_review.md`), one read-only `gpt-6-astra` xhigh turn per round. It saves each review under `/tmp/claude/astra/`.
- **Process owner:** SDLC, a new top-level showrunner, owns the design process, the guilds and the naming. When the showrunner gives its name, process questions go to SDLC directly.
- **Stack:** Markdown in Simplified Technical English (ASD-STE100), the user's rule of 2026-10-18. No forbidden words (`~/rust/nate_style/rust/forbidden-words.md`); the hooks block them.
- **Test lanes:** none. The phases write documents only.
- **Key files (read-only evidence for the designer):**
  - Prior experiments and their write-ups: `~/.claude/docs/as-built/director-model-study.md` (with `director-model-study-results.md`, `director-model-study-results.json`), `~/.claude/docs/as-built/review-regime-trial.md` (with `review-regime-trial.jsonl`), `~/.claude/docs/as-built/token-efficiency-study.md`, `~/.claude/docs/as-built/herdr-trial.md`, `~/.claude/docs/as-built/review-pool-probes.md`.
  - Lessons on measurement, in `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/`: `handoff-mend-clippy-one-compile.md` (a benchmark measured only the best case; the live gain was 5–25 min/day, so the work was rolled back), `attempts-phase8-berth-tests-under-a-second.md` (25 parallel filter runs measure contention, not one cut), `reduce-test-load.md` (only wall clock counts), `no-multi-second-tests.md`, `attempts-reader-tests-mac-under-a-second.md` (measure each cut on the Mac in turn against the version before it), `mac-pass-ends-the-test-gate.md`, `verify-runs-one-at-a-time.md`, `build-hold-release-may-never-finish.md`, `cap-repair-rounds.md`.
  - Measurement stores the environment already has: `~/.claude/scripts/buildlog/` (build, lint, test and CI timings and failures on both machines; `/build:report`, `/build:mac_report`), `~/.claude/scripts/model_study/` (one store of every model turn, outcomes and named metric queries; `~/rust/hanadocs/showrunners/natedev/model-study.md`), `~/.claude/docs/as-built/usage-ledger.md` and `~/.claude/scripts/usage/`, `~/.claude/scripts/history/history.py` (stage, phase and run timings), `~/.claude/scripts/nightly_review/` with `~/.claude/commands/nightly_review.md` and `~/.claude/commands/efficiency_next.md` (proposals ranked by measured gain, and a ledger of outcomes).
  - Tooling precedents: `crates/cargo-berth/` with `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/hook-cost.md` and `docs/cargo-berth/as-built/operations.md`; the hooks in `~/.claude/scripts/hooks/` (for example `pre-tool-use-berth.py`, `session-start-delegate-resume.py`); `crates/cargo-handler/` with `docs/cargo-handler/as-built/hierarchy.md` and `docs/cargo-handler/as-built/startup.md`.
  - The environment: `~/.claude/scripts/direct/verify.sh` (the gate every unit runs), `~/.claude/commands/build/` (build holds on natedev and the Mac), `~/.claude/commands/unit/direct.md`, `~/.claude/commands/showrunner/produce.md`, `~/.claude/docs/production_format.md`, `~/.claude/docs/decision_criteria.md` (correctness, then simple, fast, beautiful), `crates/cargo-berth/README.md` (worktree reservations).
- **Build / Test / Lint:** none. The phases change no package.
- **Invariants:**
  - The output is a design only, under `docs/experiments/`. No code, and no edit outside `docs/experiments/` and this plan.
  - The designer reads; it does not run builds, tests or experiments on natedev or the Mac.
  - At most three review rounds in all (showrunner, 2026-10-20). After the third, the design goes to the showrunner with its open findings listed.
- **Constraints from prior phases (every remaining Work Order):** none yet.

## Phases

### Phase 1 — A first draft of the experiment framework design  · status: done

#### As-built

- Two documents under `docs/experiments/` hold a design for an experiment framework. Agents use it to run experiments on any code base and on the process, and to get results that are reliable and repeatable.
- The design is a proposal only. It has no code.
- The framework follows the scientific method from question to adoption. Each section fits this environment: natedev and the Mac, build holds and load, `verify.sh`, berth, units and showrunners, and the test-time rules.
- `cargo-experiment` is a proposed binary name only. The SDLC showrunner owns the name.

**Files:**
- `docs/experiments/framework.md` — a short summary, a one-page procedure, and sections 1-13: question and hypothesis, pre-registration, baseline and control, conditions, measures and existing evidence, repeat runs and noise floor, live effect and daily value, acceptance and authority, adoption and keeping the result, repeatability, place in a production, records and tools (experiment directory, protocol and attempt fields, named states, smallest new tool set), and tooling (CLI, JSON envelope, hooks, cargo-handler view). It ends with the open questions for the user.
- `docs/experiments/worked-examples.md` — the procedure applied to five past experiments: mend and clippy in one compile, the review regime trial, cargo-berth test-time reduction, the director model study, and review-pool reset and fork probes. Section 6 is a complete pre-registered protocol for the first live experiment: review context after reset.

**Binds later work:**
- Three questions stay open for the user: which first live study to run, the showrunner spending limit, and how long to keep evidence. The review of the design sends them to the showrunner with the design.
- The named states in `framework.md` §12 and the tools in §13 are a proposal for a later build, not a contract.

**Gotchas:** The existing stores have limits. The build log has no one-minute load. No store counts process starts or user corrections. A measure that needs one of these has no source until a later build adds it.

### Phase 2 — The design passes Astra review  · status: done

#### As-built

- Both documents under `docs/experiments/` have had three `/astra_review` rounds, each followed by a revision in the same designer thread. No finding is open or in dispute.
- The first tool version supports one study design: independent same-input pairs with a positive gain. Unpaired allocation, equal-quality claims and retained-thread policy studies are unsupported.
- `framework.md` §6 holds one failure table: preflight refusal, measured environmental breach, missing measurement, missing receipt and workload failure. Only a measured, registered environmental breach permits a replacement pair.
- Each pair runs as one launcher job with separate arm receipts. A durable workload receipt is sealed before the runner's notice. Mac timing and conditions come from the Mac through `mac_run.sh`, and the receipt is copied back once.
- The shared index is published under one owner-store lock, tied to journal generations. An index from before an interrupted update answers `stale`.
- Phase elapsed time comes from `phase_started` and `phase_finished` events, joined by `phase_instance_id`. It does not come from summed stage seconds.
- The first live proposal in `worked-examples.md` §6 only exercises the framework: the history command with and without `python3 -S`, with a limit of 1,800 command seconds and 60 operator minutes in 28 days. It cannot authorize adoption.

**Files:**
- `docs/experiments/framework.md` — the revised design, sections 1–13 and three open questions for the user
- `docs/experiments/worked-examples.md` — five past experiments and the framework-exercise protocol

**Gotchas:** The round 3 fixes (failure table, index freshness, Mac receipt path, one job per pair, first-study purpose) had no further review, because of the 3-round cap. `mesh_runs.py` sends a completion notice before it saves its finished record. `buildlog/sample.py` is Linux only.

**Ruled out:** a reset-reviewer study as the first live study, because units keep their reviewer and never reset it, so its control arm is not current policy. Marking Mac measurements unsupported, because the design must fit natedev and the Mac.
