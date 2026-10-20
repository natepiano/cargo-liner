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

#### Work Order

**Goal:** `docs/experiments/framework.md` holds a complete first draft of a framework, with which agents run experiments on any of our code bases and on our process and get results that are reliable and repeatable. `docs/experiments/worked-examples.md` applies the draft to past experiments.

**Spec:**
1. Source: the user's words in **Source** above, and the showrunner, 2026-10-20: "a design only, under docs/experiments/. No code. The design covers how agents run experiments on any of our code bases and on our process: hypothesis, baseline, controls, measures, repeat runs, the noise floor, how a result is accepted, and how an accepted result is adopted and kept. It must fit our environment: natedev and the Mac, shared build locks and load, verify.sh, berth, units and showrunners, the test-time rules (no multi-second tests), and the lessons in the memory files about load and timing."
2. Use the scientific method from start to end. The design answers each item below, and each answer is specific to our environment:
   1. **Question and hypothesis.** How an agent writes a question, a falsifiable hypothesis, the change under test and the one primary measure, before any run.
   2. **Pre-registration.** What the agent records before the first run (hypothesis, measure, sample size, stop rule, acceptance rule), where it records it, and how a later reader sees that the rule did not change after the data came in.
   3. **Baseline and control.** How the baseline is the same tree and conditions minus the one change (A/B on the same commit base, paired or interleaved runs), for code (a crate on natedev or the Mac) and for process (a skill, a prompt, a model, a review regime, where agent output is not deterministic).
   4. **Conditions.** How a run records and controls machine load, build holds, concurrent units, sccache and cold or warm caches, the machine, the account and the model. When a sample is untrusted (for example, natedev load above 16 of 32).
   5. **Measures.** The primary and secondary measures for each kind of experiment (wall clock, process starts, tokens and cost, defects found, review rounds, user corrections), and which existing store supplies each one (Key files). Wall clock is the only time that counts.
   6. **Repeat runs and noise floor.** How the agent measures the noise floor from A/A runs before it reads a difference, how it picks the number of runs, and which statistic it uses (for example, a median with a bootstrap interval, a paired test). Small samples of agent runs are the usual case; say how to deal with them.
   7. **Live effect, not best case.** How a result must show the gain in real use and not only in a best-case benchmark (the mend + clippy lesson), and how an experiment estimates the gain per day or week from the stores.
   8. **Acceptance.** The rule that accepts, rejects or calls a result not proven, and who decides (the unit, the showrunner, the user) at each level of cost and risk, under `~/.claude/docs/decision_criteria.md`.
   9. **Adoption and keeping.** How an accepted result becomes the default (a config, a skill, a style rule, a memory, a plan phase), how it is recorded so that agents find and use it, and how a later check finds a regression or a result that is no longer true.
   10. **Repeatability.** What a record holds so that another agent can repeat the experiment on another day and get the same answer within the noise floor: commit, commands, conditions, raw data, analysis.
   11. **Place in the production model.** How an experiment runs as work in a unit or a showrunner: berth reservations, build holds, `verify.sh` runs one at a time, Mac slots, and how it avoids disturbing other units' work and timings.
   12. **Records and tools.** The record format and where records live, which existing stores and scripts it reuses, and the smallest new tool set a later build needs. Keep it simple: few concepts and one way to do each thing. The next agent must find an accepted result again: say where the result lives, and how a later session finds it before it runs the same experiment again (showrunner, 2026-10-20, from the user's words "if we run an experiment and we find a good result, then we're going to want to use this").
   13. **Tooling** (the user via the showrunner, 2026-10-20: tooling like cargo-berth, a CLI with commands and hooks, and maybe an experiment view in cargo-handler). Design it; do not build it.
       - **CLI:** a cargo-berth-style binary in this workspace. Commands to register a pre-registered protocol, start and record runs with their conditions, compute the A/A noise floor and the verdict, accept or reject, adopt, and find past results by subject. Define the record format and where it is stored. Use the same envelope style as cargo-berth's `--json` (`docs/cargo-berth/as-built/json-contract.md`).
       - **Hooks:** for example, a hook that shows a session the prior accepted results for the code or process it is about to change, and a hook that blocks or warns when a run's conditions break its protocol (load, a build hold, the wrong machine). Say which hook events they use and what each prints. A hook prints a line once, and only when the line is true for that case.
       - **cargo-handler view:** what an experiment view shows (open experiments, runs done and left, the noise floor, the verdict) and where it lives in the app. Keep it in simple text.
3. `docs/experiments/framework.md` holds the design. It begins with a short summary and a one-page procedure that an agent follows step by step. It ends with a list of open questions for the user.
4. `docs/experiments/worked-examples.md` applies the procedure to at least three past experiments from the Key files, among them the mend + clippy one compile and the review regime trial. For each, it shows what the framework would have required, and whether that would have changed the result or found the error sooner. It also writes one complete pre-registered protocol for a first live experiment that the user can pick to run after approval.
5. The unit director starts the designer thread with this Work Order and the Delegation Context, and records the thread id and `CODEX_HOME` in the unit notes.

**Files:**
- `docs/experiments/framework.md` — the design (new)
- `docs/experiments/worked-examples.md` — the procedure applied to past experiments, and the first live protocol (new)

**Workers:** 1 writer: the Astra designer thread holds both documents, because the examples follow the procedure and cannot split from it.
- `impl` — the Astra designer (`gpt-6-astra`, xhigh); `docs/experiments/framework.md`, `docs/experiments/worked-examples.md`
- `test` — opens as impl; no test lane

**Constraints from prior phases:** none.

**Acceptance gate:** Both documents exist and answer each item of Spec 2 in its own section. Each claim about a store, script or past experiment names the file it comes from. The documents hold no forbidden words. The unit notes hold the designer thread id and `CODEX_HOME`.

### Phase 2 — The design passes Astra review  · status: todo

#### Work Order

**Goal:** After at most three `/astra_review` rounds, each followed by a revision in the same designer thread, the design has no open correctness finding, and the showrunner has the design with any open findings listed.

**Spec:**
1. A round: run `/astra_review docs/experiments/framework.md` with the focus "an experiment framework for our agents on natedev and the Mac; check it against the Key files and the worked examples in docs/experiments/worked-examples.md; correctness first, then simplicity". Then resume the designer thread with the review in full. The designer revises both documents and says, for each finding, what it changed or why it does not agree.
2. The unit director rules on each disagreement. A finding about correctness or about a fact in a Key file is settled from the file, not by the designer's view.
3. Stop after a round with no correctness finding, or after the third round.
4. Send the showrunner the paths of both documents, a short summary, the round count and each open finding with its ruling.

**Files:**
- `docs/experiments/framework.md` — revised per each review
- `docs/experiments/worked-examples.md` — revised per each review

**Workers:** 1 writer: the same Astra designer thread revises both documents; the reviewer only reads.
- `impl` — the Astra designer thread from phase 1; `docs/experiments/framework.md`, `docs/experiments/worked-examples.md`
- `test` — opens as impl; no test lane

**Constraints from prior phases:** The designer thread from phase 1 is the only designer session; resume it and never start a second one.

**Acceptance gate:** The last review has no open correctness finding, or three rounds are done and each open finding is listed with a ruling. The documents hold no forbidden words. The showrunner has the message of Spec 4.
