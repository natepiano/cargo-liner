# Published practice compared with the local experiment design

Date: 2026-10-30. Author: experiments-unit. Scope: read and report only. This report changes no design, tool, or default.

The local design is `docs/experiments/framework.md` (696 lines) and `worked-examples.md`. This report compares it with published practice from four fields:

1. Command and code benchmarks.
2. Online controlled experiments (A/B tests).
3. Evaluation of language-model agents.
4. Pre-registration and decision records.

It ends with ranked options to adapt.

## Summary

The local design agrees with published practice on the statistical core:

- Paired comparison on the same input.
- An A/A stage before A/B.
- Rules fixed before the first run.
- No stop because a result looks good.
- Every attempt kept.
- Correctness checks that are separate from the speed measure.

The design differs from published practice on its record and tooling layer. Published tools keep a plain run log and a short decision record. The local design adds these items, and no published source in this report uses them:

- A hash-chained journal.
- A salted commitment for the blind labels.
- An owner store with a derived index and publication checks.
- A Mac receipt-transfer protocol.
- Two hooks and an app view.

That layer is most of the design's size and most of its new concepts. It also holds state in more than one place: the owner store, the derived index, and a Mac mirror. The user's rule of 2026-10-30 does not allow that without approval.

The first live study is a command-timing study (`history.py` with and without `python3 -S`). Published tools already do that kind of work without a new binary.

## 1. Command and code benchmarks

| Practice | Source | Local design |
| --- | --- | --- |
| Do warmup runs before measurement. Run a prepare command before each timed run to reset state. Export all runs as JSON. Warn on statistical outliers. | hyperfine [1] | Same rules (section 4, cold and warm). The design specifies a new adapter to do this. |
| Small setup details change results: environment size and link order changed measured speed on every machine and compiler tested. None of 133 surveyed papers controlled for this. The remedy is setup randomization. | Mytkowicz et al., ASPLOS 2009 [2] | Path is a controlled factor (section 3). Environment size and other setup details are not randomized. |
| Variation comes from several levels: build, process, and iteration. Put repeats at the level with the most variation. | Kalibera and Jones, ISMM 2013 [3] | The A/A stage measures total noise. It does not split the noise by level. |
| On a shared or busy machine, instruction counts (cachegrind) moved less than 0.04% while wall-clock medians moved about 150%. Use counts to gate CI, and still report wall time. | tak project [4]; Python Speed [5]; rustls CI bench [6] | Wall clock is the only time measure for a claim (user rule, section 5). The design has no low-noise screening measure. |
| Run the A and B versions repeatedly on the same lab device and combine the runs. | Chromium Pinpoint [7] | Same rule: pairs on the same machine, natedev and the Mac reported separately. |

## 2. Online controlled experiments

| Practice | Source | Local design |
| --- | --- | --- |
| Run A/A tests to check the system. Check sample ratio mismatch (SRM) on every experiment. A mismatch shows a defect in allocation or logging. | Kohavi, Tang, Xu [8] | A/A stage is present. Unequal invalid counts by arm give `not_proven` (section 4). This is the same check under a different name. |
| Guardrail measures are things the change must not make worse. A breach stops the experiment. | Kohavi et al. [8] | Correctness checks and cost limits (sections 1, 6, 8). Same practice. |
| Twyman's law: a result that looks unusual is usually an error. Check the instruments before you accept it. | Kohavi et al. [8]; Wikipedia summary [9] | No rule makes a large unexpected gain start a check of the instruments. |
| Repeated looks at the data increase false positives. A group sequential test with alpha spending (Lan and DeMets) permits a few planned looks with a valid stop. | Spotify engineering [10]; anytime-valid confidence sequences [11] | Early stop for a good result is forbidden (section 2). This is correct for a fixed design, but it spends all runs even when the effect is large. |

## 3. Evaluation of language-model agents

| Practice | Source | Local design |
| --- | --- | --- |
| Compare on the same questions and analyze paired differences. This is a variance reduction at no cost, because difficulty is shared. | Anthropic, statistical approach to evals [12]; Miller 2024 [13] | Same rule (sections 3 and 6). |
| Cluster standard errors on the unit that is not independent. Clustered errors can be more than three times the naive errors. | [12], [13] | The design joins related pairs into groups and keeps one pair per group (section 3). That rule is valid, but it discards data. The published practice keeps the data and widens the interval. |
| Do a power analysis before the study. Small sets cannot show small differences. | [12] | Fixed defaults of 8 A/A and 12 A/B pairs (section 6), with a note to increase them. No step computes the count from the A/A variance and M. |
| Run several trials per task, because outputs change between runs. Report pass@k or pass^k. Single-run SWE-bench scores moved 2.2 to 6.0 points between runs of the same setup, also at temperature 0. | Anthropic, demystifying evals [14]; SWE-bench variance study [15]; ICC study [16] | One output per arm per task. Repeat trials inside a task are not part of the first version. |
| Start each trial from a clean environment. Shared state gave one agent an unearned advantage (it read git history from earlier trials). | [14] | Independent context per arm (section 3). Same practice. |
| Grade the outcome, not the path. Prefer deterministic graders, then model graders, then people. Read many transcripts to check the graders. | [14]; Husain [17] | Blind rubric scoring and confirmed defects (section 3). The design has no step that reads transcripts to check the scorer. |
| Do error analysis before you write the measure: read 20 to 100 real traces and sort the failures. Start with 20 to 50 tasks drawn from real failures. | Husain [17]; [14] | The question comes first (section 1). The design does not tell the author to read real failures before choosing the primary measure. |
| A model judge prefers the first answer and its own model family. Swap the order and count only results that stay the same. Use a judge from another family. Check the judge against human labels. | Zheng et al. via [18]; self-preference study [19]; [14] | Concealed X/Y labels with a salted SHA-256 commitment and separated access (section 3). This prevents a judge from seeing the arm label. It does not control order bias or family bias. |
| A pinned repository commit and a full log make a run repeatable. A log alone is not proof of correct results. | Inspect evals census [20]; Inspect guide [21] | Same rules (section 10). |

## 4. Pre-registration and decision records

| Practice | Source | Local design |
| --- | --- | --- |
| Register the plan before data. Allow changes, but report each deviation in a table with its reason. Only 2 of 27 pre-registered studies had no deviation. | Best Laid Plans [22]; OSF Transparent Changes [23]; UZH summary [24] | Every rule change needs a new protocol revision, and the old comparison closes as `not_proven` (section 2). This is stricter than published practice. |
| A decision record has a status (proposed, accepted, rejected, superseded). Never delete a record. A superseded record links to its successor. Keep records in the repository next to the code. | Nygard via Fowler [25]; GDS standard [26]; adr-tools [27] | Same statuses and `supersede` (sections 8, 9, 13). The design keeps records in a separate store at `~/.local/state/experiments/`, not in the repository. |

## Ranked options to adapt

The options are ranked by correctness first, then by the three gods (Simple, Fast, Beautiful). Each option names what it removes or adds.

1. **Records in git, not a new store.** Put each protocol and result as a file in the owning repository (decision-record practice [25]–[27]). A commit gives the registration time, the content hash, and the history. A pushed commit also gives a reference that this machine alone cannot change. This removes the owner store, the hash-chained journal, the derived index, the publication and freshness checks, and the Mac mirror. That removes three copies of the same state. Search uses `git grep` over the record directory.
2. **Run the first study with hyperfine and one analysis script.** The `history.py -S` study is a command timing [1]. Run hyperfine in alternating A/B rounds with `--warmup`, `--prepare`, and `--export-json`. Then run one script that computes the paired median, the order-statistic interval, and N from section 6. Build `cargo-experiment` only if a later study needs something these two cannot do. This also tests the statistical rules before the design spends work on tooling.
3. **Compute the sample count from the A/A stage.** Use the A/A variance and M to compute the A/B count before the A/B stage starts. Record the count in the protocol [12]. This keeps the fixed-count stop rule. It replaces the defaults of 8 and 12, which have no stated precision basis.
4. **Use the published agent-evaluation practice for review and model studies.**
   - Run several trials per task [14], [15].
   - Grade outcomes with deterministic checks first [14].
   - Read transcripts to check the scorer [14], [17].
   - For a model judge, swap the answer order and use a judge from another model family [18], [19].
   - Keep the blind labels, but drop the salted commitment and the separated-access rule. A plain random mapping kept in the operator's record is sufficient for a single-user system.

   This adds trial repeats and removes a cryptographic step that controls a risk no source reports.
5. **A deviation table in place of a full re-registration for small changes.** Permit a recorded change that does not touch the claim, the primary measure, or the analysis. Each change goes in a table with its reason and time [22], [23]. A change to those three items still needs a new protocol. This keeps a study alive after a small setup fix.
6. **Two low-cost checks against false results.**
   - Randomize setup details that can bias timing, such as environment size and working-directory name, for each run [2].
   - Apply Twyman's law: a gain much larger than M starts a check of the instruments before `accepted` [8].
7. **Instruction counts as a screening measure on a busy natedev.** Use cachegrind counts to screen CPU-bound code changes while other units load the machine [4]–[6]. A claim still needs wall clock (user rule). This lets a study screen changes without a build hold, which supports the CPU-cost priority.
8. **A planned early stop for expensive agent studies.** Use a group sequential design with two or three looks and alpha spending [10]. A large effect then stops the study early with a valid result. This adds one concept, so do it only for agent studies where each pair takes minutes or hours.
9. **Clustered intervals in a later version.** Compute cluster-level intervals [12], [13] in place of one pair per dependence group. This keeps all data from related tasks. Leave it until a study needs it.

Options 1 and 2 together remove most of section 12 and section 13 before the first study. The hooks and the cargo-handler view then wait until git holds real records to show.

## Sources

1. hyperfine documentation and repository: https://github.com/sharkdp/hyperfine ; https://docs.rs/crate/hyperfine/latest
2. Mytkowicz, Diwan, Hauswirth, Sweeney, "Producing Wrong Data Without Doing Anything Obviously Wrong!", ASPLOS 2009: https://sape.inf.usi.ch/publications/asplos09
3. Kalibera and Jones, "Rigorous Benchmarking in Reasonable Time", ISMM 2013: https://kar.kent.ac.uk/33611/ ; libkalibera notes: https://www.kvakil.me/posts/2022-04-01-using-libkalibera.html
4. tak methodology (one author's measurements): https://tak.jdx.dev/guide/methodology
5. Python Speed, "Consistent benchmarking in CI": https://pythonspeed.com/articles/consistent-benchmarking-in-ci
6. rustls CI bench: https://github.com/rustls/rustls/blob/main/ci-bench
7. Chromium perf try bots (Pinpoint): https://chromium.googlesource.com/chromium/src/+/ab9fc65b771f/docs/speed/perf_trybots.md
8. Kohavi, Tang, Xu, *Trustworthy Online Controlled Experiments*, chapter "Sample Ratio Mismatch and Other Trust-Related Guardrail Metrics": https://www.cambridge.org/core/books/trustworthy-online-controlled-experiments/sample-ratio-mismatch-and-other-trustrelated-guardrail-metrics/8DBB0F59AC7729D7BC6B94690DB9CCD5
9. Twyman's law: https://en.wikipedia.org/wiki/Twyman%27s_law
10. Spotify, "Choosing a Sequential Testing Framework": https://engineering.atspotify.com/2023/3/choosing-sequential-testing-framework-comparisons-and-discussions
11. "Anytime-Valid Confidence Sequences in an Enterprise A/B Testing Platform": https://arxiv.org/pdf/2302.10108
12. Anthropic, "A statistical approach to model evaluations": https://www.anthropic.com/research/statistical-approach-to-model-evals
13. Miller, "Adding Error Bars to Evals": https://arxiv.org/pdf/2411.00640
14. Anthropic, "Demystifying evals for AI agents" (January 2026): https://anthropic.com/engineering/demystifying-evals-for-ai-agents
15. SWE-bench run-to-run variance study: https://arxiv.org/html/2602.07150v2
16. "Stochasticity in Agentic Evaluations: Quantifying Inconsistency with Intraclass Correlation": https://arxiv.org/html/2512.06710v1
17. Husain, "LLM Evals: Everything You Need to Know" and "Error analysis before writing tests": https://hamel.dev/blog/posts/evals-faq/ ; https://hamel.dev/notes/llm/officehours/erroranalysis.html
18. Zheng et al., "Judging LLM-as-a-Judge with MT-Bench" (summary): https://tailoredai.substack.com/p/judging-llm-as-a-judge-with-mt-bench
19. "Do LLM Evaluators Prefer Themselves for a Reason?": https://arxiv.org/pdf/2504.03846
20. "What Does an Evaluation License? A Commit-Bound Census of Claim Replay in Inspect Evals": https://arxiv.org/pdf/2608.19269
21. Inspect AI guide (CASRAI): https://casrai.org/guides/inspect-ai-evaluation-framework-uk-aisi
22. "Best Laid Plans: A Guide to Reporting Preregistration Deviations": https://osf.io/et6km ; https://doi.org/10.1177/25152459231213802
23. OSF Transparent Changes template: https://osf.io/yrvcg/
24. University of Zurich open-science newsletter: https://www.psychology.uzh.ch/en/areas/open-science/news/nl-005.html
25. Fowler, "Architecture Decision Record": https://www.martinfowler.com/bliki/ArchitectureDecisionRecord.html
26. GDS Way, architecture decisions: https://gds-way.digital.cabinet-office.gov.uk/standards/architecture-decisions.html
27. adr-tools: https://www.hascode.com/managing-architecture-decision-records-with-adr-tools/

## Source limits

- Some sources were read only as excerpts or summaries: [8], [9], [18], [21], and the details of [1]. The original Nygard article and the Microsoft SRM paper were not opened.
- [4] is one author's measurements.
- [15] and [16] are recent preprints.
- No source gave Pinpoint's statistical test.
