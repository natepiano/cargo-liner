# berth-parallel

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15 14:53 EDT

One unit director should be able to run two phases side by side, each with its own reservation and checkpoint (produce.md's "parallel by default"). Today one director session plus one active reservation folds every later phase's files into that reservation (SessionMappingOrSingleActive, cli.rs:1088 check and :1389 drift), so they release with the first phase's checkpoint. Seen 2026-10-15 in tool-based-ui's info-panels plan, where Phases 16 and 20 had to wait for 15. The director runs check once per phase under CARGO_BERTH_SESSION_ID=<director sid>.phase-<N>, with paths split by each phase's Work Order. Two engine changes needed: (a) a check under a session id with no mapping opens a new reservation in the same coordination run instead of joining the run's only active one; (b) drift from the director's own Bash calls never widens one phase's reservation with another phase's files. Fix fast and install, test that the fix works first, robust testing after.
