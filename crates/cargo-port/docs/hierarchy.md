# Project Type Hierarchy Refactor

## Motivation

The current hierarchy uses a generic `RustProject<Kind: CargoKind>` with a trait
and two marker types (`Workspace`, `Package`). This causes:

- **5 `RootItem` variants** instead of 3 (Workspace/Package doubled by worktree groups)
- **Generic propagation** into `WorktreeGroup<Kind>`, forcing parallel match arms everywhere
- **63% of call sites are kind-agnostic** but pay the 5-variant complexity tax
- **`project.rs` is 2220 lines** with 6+ type clusters and 3+ mixed domains

## Proposed Type Hierarchy

```
RootItem                           (3 variants)
├── Rust(RustProject)
├── NonRust(NonRustProject)
└── Worktrees(WorktreeGroup)

RustProject                        (enum + delegation API)
├── Workspace(WorkspaceProject)
└── Package(PackageProject)

WorkspaceProject                   (struct, Deref → RustInfo)
├── path: AbsolutePath
├── name: Option<String>
├── rust: RustInfo
└── groups: Vec<MemberGroup>

PackageProject                     (struct, Deref → RustInfo)
├── path: AbsolutePath
├── name: Option<String>
└── rust: RustInfo

RustInfo                           (struct, Deref → ProjectInfo)
├── info: ProjectInfo
├── cargo: Cargo
├── vendored: Vec<PackageProject>
├── worktree_name: Option<String>
└── worktree_primary_abs_path: Option<AbsolutePath>

NonRustProject                     (struct, Deref → ProjectInfo)
├── path: AbsolutePath
├── name: Option<String>
└── info: ProjectInfo

ProjectInfo                        (struct — shared metadata for ALL projects)
├── disk_usage_bytes: Option<u64>
├── git_info: Option<GitInfo>
├── visibility: Visibility
└── worktree_health: WorktreeHealth

WorktreeGroup                      (enum — compile-time kind safety)
├── Workspaces { primary: WorkspaceProject, linked: Vec<WorkspaceProject> }
└── Packages { primary: PackageProject, linked: Vec<PackageProject> }

MemberGroup                        (enum — unchanged)
├── Named { name: String, members: Vec<PackageProject> }
└── Inline { members: Vec<PackageProject> }
```

### Design Decisions

- **`path` and `name` stay as direct fields** on each project type, not inside
  `ProjectInfo`. Existing `info_mut()` APIs hand out `&mut ProjectInfo` for async
  metadata updates — moving identity fields there would let callers accidentally
  mutate lookup keys and break ordering invariants in `ProjectList`.

- **`worktree_health` stays on `ProjectInfo`**, not `RustInfo`. Non-Rust git repos
  can be worktrees. It's a git concept, not a Rust concept.

- **No traits.** `CargoKind` and `InfoProvider` are eliminated. Type distinctions
  are concrete structs and enums. The only trait impls are standard `Deref`.

## Deref Chain

Enables uniform field access without matching:

```
WorkspaceProject  ──Deref──►  RustInfo  ──Deref──►  ProjectInfo
PackageProject    ──Deref──►  RustInfo  ──Deref──►  ProjectInfo
NonRustProject    ──Deref──►  ProjectInfo
```

Any code holding any project type can call `.info.visibility`, `.info.git_info`,
etc. directly via Deref. Rust-specific fields like `.cargo` are available on
anything that derefs to `RustInfo`. Workspace-specific `.groups` only exists on
`WorkspaceProject`.

## Delegation APIs

The `RustProject` enum and `WorktreeGroup` enum need forwarding methods since
callers often hold the enum, not the concrete type. These are trivial 2-arm
matches the compiler inlines.

### `RustProject` delegation

```rust
impl RustProject {
    fn path(&self) -> &Path;
    fn name(&self) -> Option<&str>;
    fn info(&self) -> &ProjectInfo;
    fn info_mut(&mut self) -> &mut ProjectInfo;
    fn rust_info(&self) -> &RustInfo;
    fn cargo(&self) -> &Cargo;
    fn vendored(&self) -> &[PackageProject];
    fn worktree_name(&self) -> Option<&str>;
    fn worktree_primary_abs_path(&self) -> Option<&Path>;
    fn package_name(&self) -> PackageName;
    fn display_path(&self) -> DisplayPath;
    fn root_directory_name(&self) -> RootDirectoryName;
    fn visibility(&self) -> Visibility;
    fn worktree_health(&self) -> WorktreeHealth;
    fn disk_usage_bytes(&self) -> Option<u64>;
    // kind-specific (returns None for packages)
    fn groups(&self) -> Option<&[MemberGroup]>;
    fn groups_mut(&mut self) -> Option<&mut Vec<MemberGroup>>;
}
```

### `WorktreeGroup` delegation

```rust
impl WorktreeGroup {
    fn primary_path(&self) -> &Path;
    fn primary_info(&self) -> &ProjectInfo;
    fn primary_rust_info(&self) -> &RustInfo;
    fn visibility(&self) -> Visibility;
    fn live_entry_count(&self) -> usize;
    fn renders_as_group(&self) -> bool;
    fn single_live(&self) -> Option<&RustProject>;
    // For kind-specific access, callers match on the variant
}
```

## Module Structure

`project.rs` (2220 lines) splits into `project/` per the style guide — it hits
all 4 split criteria: 6+ type clusters, 3+ mixed domains, >500 lines,
independently testable sections.

```
project/
  mod.rs              facade: mod declarations + pub use re-exports
  paths.rs            AbsolutePath, DisplayPath, RootDirectoryName, PackageName
  git.rs              GitOrigin, GitInfo, GitPathState, WorkflowPresence,
                      GitRepoPresence, all detection functions
  cargo.rs            ProjectType, ExampleGroup, CargoParseResult,
                      from_cargo_toml, detect_types, collect_examples
  info.rs             ProjectInfo, Visibility, WorktreeHealth
  rust_info.rs        RustInfo, Cargo struct, Deref → ProjectInfo
  workspace.rs        WorkspaceProject, Deref → RustInfo
  package.rs          PackageProject, Deref → RustInfo
  rust_project.rs     RustProject enum + delegation API
  non_rust.rs         NonRustProject, Deref → ProjectInfo
  worktree_group.rs   WorktreeGroup enum + delegation API
  root_item.rs        RootItem enum + traversal helpers
  member_group.rs     MemberGroup enum + count_rs_files_recursive
```

### Module boundaries rationale

- **`paths.rs`** — domain cohort: peer newtypes, no single anchor
- **`git.rs`** — domain cohort: multiple peer types serving one domain (~733 lines
  but cohesive; splitting further would create artificial boundaries)
- **`cargo.rs`** — anchor: `CargoParseResult` + `from_cargo_toml` + target collection
- **`info.rs`** — anchor: `ProjectInfo` + supporting enums shared by all project types
- **`rust_info.rs`** — anchor: `RustInfo` (shared Rust-project data) + `Cargo` struct
  (a field type of `RustInfo`)
- **`workspace.rs`** / **`package.rs`** — one anchor type each; small files but each
  carries its own Deref impl, doc comments, and tests
- **`rust_project.rs`** — anchor: `RustProject` enum with substantial delegation API
- **`non_rust.rs`** — anchor: `NonRustProject`
- **`worktree_group.rs`** — anchor: `WorktreeGroup` enum with delegation API
- **`root_item.rs`** — anchor: `RootItem` enum (~370 lines of traversal logic)
- **`member_group.rs`** — anchor: `MemberGroup` enum

`project_list.rs` stays as a peer file alongside `project/` — it's one anchor
type in one domain. After the refactor simplifies match arms (5 → 3 variants),
it should shrink well under 500 lines.

## What Changes

| Aspect | Before | After |
|---|---|---|
| RootItem variants | 5 | 3 |
| WorktreeGroup | generic struct | enum with 2 variants |
| RustProject | generic struct | enum wrapping 2 concrete structs |
| CargoKind trait | exists | eliminated |
| InfoProvider trait | exists | eliminated |
| Shared field access | per-type accessor methods | Deref chain |
| `.groups()` safety | compile-time (generic) | compile-time (only on WorkspaceProject) |
| WorktreeGroup kind safety | compile-time (generic) | compile-time (enum variants hold concrete types) |
| `worktree_health` | on ProjectInfo | stays on ProjectInfo (git concept, not rust-specific) |
| `path`, `name` | on each project type | stays on each project type (immutable identity) |
| `project.rs` | 2220-line monolith | 13 focused submodules |

## What Doesn't Change

- `MemberGroup` enum (Named/Inline with package members)
- `Cargo`, `GitInfo`, `Visibility`, `WorktreeHealth`, `AbsolutePath` — leaf types unchanged
- `ProjectList` wrapping `Vec<RootItem>` — stays as peer file
- `ProjectInfo` metadata fields — `disk_usage_bytes`, `git_info`, `visibility`, `worktree_health`

## Migration Strategy

### Phase 1: Module split (no type changes)

Split `project.rs` into `project/` submodules with the current types. This
isolates the structural change from the semantic change, making each step
independently verifiable.

1. Create `project/mod.rs` as facade
2. Move each type cluster to its submodule
3. Wire up `pub use` re-exports so external call sites don't change
4. Verify: `cargo build && cargo +nightly fmt && cargo nextest run`

### Phase 2: New type definitions

1. Create `ProjectInfo` with `path`, `name` removed (metadata-only, as today)
2. Create `RustInfo` struct with rust-specific shared fields
3. Create `WorkspaceProject` and `PackageProject` as concrete structs with
   `path`, `name` as direct fields
4. Create `RustProject` enum wrapping them + delegation API
5. Redefine `WorktreeGroup` as enum with `Workspaces`/`Packages` variants + delegation API
6. Implement Deref chain
7. Reduce `RootItem` to 3 variants
8. Remove `CargoKind` trait, `InfoProvider` trait, old marker types

### Phase 3: Migrate call sites

Starting from the leaves and working up:

1. **ProjectInfo access** — update `.visibility()`, `.git_info()`, `.disk_usage_bytes()`
   call sites (most should just work via Deref)
2. **RustInfo access** — update `.cargo()`, `.vendored()`, `.worktree_name()` etc.
3. **Kind-agnostic RootItem matches** (~38 sites) — collapse 5-arm matches to 3-arm
4. **Kind-specific sites** (~22 sites) — update to match on `RustProject::Workspace`
   or `WorktreeGroup::Workspaces` to access `.groups()`
5. **Construction sites** (scan, discovery) — build new types instead of generic

### Phase 4: Validation

1. `cargo build && cargo +nightly fmt`
2. `cargo nextest run` — full test suite
3. `cargo mend --fail-on-warn` — visibility audit
4. Verify worktree detection and display (non-Rust and Rust worktrees)
5. Verify workspace member navigation (`.groups()` paths)
6. Manual UI smoke test — detail pane, render paths, search
