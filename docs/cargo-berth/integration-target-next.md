# integration-target — Next

## Items to consider

- [ ] **A branch cut from an integration branch enrolls toward that branch**
  - Target: `crates/cargo-berth/src/worktree/enrollment.rs` (automatic target selection in `read_enrollment_target`)
  - Why needed: a new branch made from an integration branch with no pinned target auto-enrolls toward the repository trunk, so its reservation holds the whole integration branch's diff against `main` and overlaps every lane touching those files.
  - Completion condition: a branch created from integration branch I with no pinned target enrolls with target I, and its reservation holds only its own work against I.
  - Revealed by: Phase 4
