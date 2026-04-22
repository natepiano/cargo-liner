# Cargo Metadata — Answers to Implementation Questions

Answers to the five clarifying questions raised from reading `docs/cargo_metadata.md` and `docs/cargo_metadata_impl.md`. Where an answer changes behavior, the design or impl plan will be updated to match.

## Q1 — Phase 0 shape (Keyed/Counted split vs PhaseExpectation wrapper)

**Authoritative: the Keyed/Counted enum split.**

```rust
pub enum PhaseState<K> {
    Keyed(KeyedPhase<K>),
    Counted(CountedPhase),
}

pub struct KeyedPhase<K> {
    pub expected:    Option<HashSet<K>>,   // None = Unknown (not yet initialized)
    pub seen:        HashSet<K>,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}

pub struct CountedPhase {
    pub expected:    Option<usize>,        // None = Unknown
    pub seen:        usize,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}
```

There is no `PhaseExpectation<K>` enum. The Testing section's references to `PhaseExpectation::Unknown` / `Set(empty)` / `Count` are stale from an earlier revision that was superseded by the enum split. Mental translation:

- `PhaseExpectation::Unknown` → `KeyedPhase.expected = None` **or** `CountedPhase.expected = None`.
- `PhaseExpectation::Set(empty)` → `KeyedPhase.expected = Some(HashSet::new())` (distinct from `None`: there's nothing to see, not "haven't decided yet").
- `PhaseExpectation::Count(n)` → `CountedPhase.expected = Some(n)`.

The completion helper is the one in the design plan:

```rust
pub fn is_complete<K: Hash + Eq>(s: &PhaseState<K>) -> bool {
    match s {
        PhaseState::Keyed(k)   => matches!(&k.expected, Some(e) if k.seen.len() == e.len()),
        PhaseState::Counted(c) => matches!(c.expected, Some(n) if c.seen == n),
    }
}
```

The design plan will be updated to purge remaining `PhaseExpectation` references in the Testing section.

## Q2 — `PhaseState<!>` vs `PhaseState<()>` for `lint_startup`

**Use `PhaseState<()>`.**

`!` is unstable on stable Rust and would force `#![feature(never_type)]`, which is not worth it for an uninhabited-key marker. `()` compiles on stable and has the same semantic effect — the `CountedPhase` variant ignores the type parameter entirely, so the chosen `K` is cosmetic. The one place `!` appears in the design plan is a mistake and will be corrected to `()`.

Implementation note: the enum split means `CountedPhase` doesn't actually use `K` — it carries no `K`-typed fields. The `K` on `PhaseState` is only there for the `Keyed` variant. So `PhaseState<()>` is a perfectly sensible way to spell "this tracker is Counted, key type is irrelevant."

## Q3 — `TargetDirIndex::siblings` `exclude` parameter

**Keep the `exclude` parameter; `build_clean_plan` passes `selection_set` into it.**

The `exclude` arg is the idiomatic plumbing for selection-aware sibling lookups. `build_clean_plan` computes `selection_set = {primary} ∪ linked` once and passes it as `exclude` to `siblings(target_dir, selection_set)`. The result is the list of genuine collateral — projects that share the target dir but are not part of the user's selection — suitable for `affected_extras` directly.

This keeps `build_clean_plan` as orchestration (computing selection, assembling the plan) and `TargetDirIndex::siblings` as a generic accessor (given a target dir and an exclusion list, return the remaining members). No selection knowledge leaks into the index, but the index supports the selection-exclusion pattern cleanly.

Equivalent pseudocode:

```rust
let selection_set: HashSet<_> = /* {primary} ∪ linked */;
let mut plan = CleanPlan::default();
for target_dir in unique_target_dirs(&selection_set, store) {
    let siblings = index.siblings(&target_dir, &selection_set.iter().collect::<Vec<_>>());
    plan.affected_extras.extend(
        siblings.iter().filter(|m| m.kind == MemberKind::Project).map(|m| m.project_root.clone())
    );
    // ...
}
```

## Q4 — Step 1 scope

**Keep Step 1 as one PR if it's comfortable; split if it starts to feel unwieldy. Concrete split option below.**

Step 1 is large because it's all pure plumbing with no consumers — each piece is trivially verifiable in isolation but the whole stack has to exist before Step 2 can do anything useful. The size reflects that reality, not over-scoping.

If the PR starts feeling too big to review confidently, split into **Step 1a** and **Step 1b**:

- **Step 1a:** `cargo_metadata` dep, `WorkspaceMetadataStore` + types, `App::resolve_*` helpers, `BackgroundMsg::CargoMetadata`, fingerprint (content hash + dispatch-generation), race guard, `metadata` phase wired into the tracker, observability toasts. Dispatch happens on initial scan only.
- **Step 1b:** Watcher classifier extension + ancestor `.cargo/` directory watch-set subsystem. This adds refresh-on-edit behavior on top of the Step 1a plumbing.

Rationale for the split boundary: Step 1a is self-contained — metadata gets fetched once at startup, never refreshed. Boring, works, ships. Step 1b layers the refresh behavior on top. Each half is reviewable independently.

Default: one PR. Split only if the reviewer (you, or the diff-size gut check) flags it. The impl plan will be updated with this split as an explicit option.

## Q5 — `resolve_target_dir` signature

**Let it accept any project path, not just a workspace root.**

Updated signature:

```rust
impl App {
    /// Returns the resolved `target_directory` for the workspace that contains
    /// `path`, or `None` if no containing workspace has a snapshot yet. Accepts
    /// any path: workspace root, workspace member, worktree entry, vendored
    /// crate root, etc.
    pub fn resolve_target_dir(&self, path: &AbsolutePath) -> Option<&AbsolutePath>;
}
```

At Step 2 call sites (`query.rs:186`, `watcher.rs:542`, scan/lint skip-walks), callers have project paths and would otherwise need to do their own "walk up to find the owning workspace root" dance. That's friction for every caller and an easy place for subtle bugs (e.g., one caller uses `PathBuf::parent` directly, another uses an existing helper, they diverge on symlinks). Centralizing the lookup in the store keeps resolution consistent.

Implementation sketch:

```rust
pub fn resolve_target_dir(&self, path: &AbsolutePath) -> Option<&AbsolutePath> {
    let workspace_root = self.store.containing_workspace_root(path)?;
    self.store.by_root.get(workspace_root).map(|snap| &snap.target_directory)
}
```

`containing_workspace_root` walks ancestors of `path` and checks `by_root` membership. Worst case O(tree_depth) per call; for ~20 workspaces with typical depth, this is a few HashMap lookups and negligible. Cache later if profiling shows it hot.

Similarly `resolve_metadata(&self, handle: &WorkspaceMetadataHandle)` stays as-is — handles already carry the workspace root, so no lookup is needed.

The design plan's "Access pattern" section will be updated to reflect this signature.

## Summary of Doc Updates Triggered

Once implementation starts, the following doc edits land as part of the first commit in `enh/cargo-metadata`:

1. **Design plan — Testing section:** replace `PhaseExpectation::*` references with the Keyed/Counted shape.
2. **Design plan — Phase 0 code sketch:** change the one stray `PhaseState<!>` to `PhaseState<()>`.
3. **Design plan — Access pattern:** update `resolve_target_dir` signature to accept any project path.
4. **Impl plan — Step 1:** add the Step 1a / 1b split as an explicit option; keep single-PR as default.

None of these change intended behavior — they reconcile wording across revisions.
