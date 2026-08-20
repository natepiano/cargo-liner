# cargo-liner consolidation

Consolidate `cargo-port`, `cargo-mend`, and a future ratatui replacement for the
`cargo-tile` shell scripts into a single workspace repo, `natepiano/cargo-liner`,
with `tui_pane` promoted from a child of `cargo-port` to a peer crate.

Both existing GitHub repos are archived in place with a `MOVED →` description.
Git history from both is grafted into the new repo, so blame survives.

**Status (2026-08-20):** Phases 0–2 complete. cargo-port is fully migrated: CI
is green on all eight jobs, every local gate passes on stable 1.98, and
`natepiano/cargo-port` is archived with a move notice. `cargo-mend` is untouched
and still develops in its own repo — Phase 3 moves it next. One item remains
open and it is not a cargo-liner defect: the shared release skill cannot
complete a dry-run for any project using `[[publish_path_pins]]` (row 16).

## Target layout

```
~/rust/cargo-liner/                    → github.com/natepiano/cargo-liner
  Cargo.toml                           virtual manifest, members = ["crates/*"]
  Cargo.lock                           one lock for the workspace
  .cargo/config.toml                   [env] RUSTC_BOOTSTRAP = "1"   (added in Phase 3)
  rustfmt.toml  taplo.toml  .gitignore
  LICENSE-MIT  LICENSE-APACHE
  README.md                            monorepo index (hana's is 215 bytes; match that)
  .github/workflows/ci.yml             one workflow, path-filtered per tool
  .claude/config/release.toml
  .claude/scripts/release/install_verify.sh
  docs/                                cross-tool docs only
  crates/
    cargo-port/    bin   0.7.0-dev     src/ tests/ scripts/ assets/ docs/ README CHANGELOG
    tui_pane/      lib   0.7.0-dev     src/ tests/ themes/   README CHANGELOG
    cargo-mend/    bin  0.19.0-dev     src/ tests/ build.rs docs/ README CHANGELOG
    <tile tool>/   bin                 Phase 6
```

Modeled on `natepiano/hana`: `crates/*` members, `[workspace.package]` inheritance
for `authors`/`edition`/`license`/`repository`, a shared `[workspace.lints]`, and a
per-crate `homepage` pointing at `https://github.com/natepiano/cargo-liner/tree/main/crates/<name>`.

Every crate keeps its own `README.md`, `CHANGELOG.md`, and its **own independent
version**. No renumbering — `cargo-port` stays at `0.7.0-dev`, `cargo-mend` at
`0.19.0-dev`, `tui_pane` at `0.7.0-dev`, so crates.io continuity is unbroken for
all three published crates.

## Release strategy

Per-crate, one at a time, via the `/release` command's single-package mode:

```
/release cargo-port 0.7.0        → branch release-cargo-port-0.7.0, tag cargo-port-v0.7.0
/release cargo-mend 0.19.0       → branch release-cargo-mend-0.19.0, tag cargo-mend-v0.19.0
/release tui_pane  0.7.0         → branch release-tui_pane-0.7.0,   tag tui_pane-v0.7.0
```

- **No `workspace_publish`, no `[[publish_phases]]`.** Both force every crate to
  ship together, which is what independent cadences must avoid. Single-package
  mode is documented as incompatible with `[[publish_phases]]`, so the choice is
  forced anyway.
- **`[[publish_path_pins]]` for `tui_pane`.** On `main`, the root
  `[workspace.dependencies]` entry is `tui_pane = { path = "crates/tui_pane" }`
  with no version. `cargo publish` rejects a path-only dep, so the release branch
  rewrites it to the last published version just before publishing and commits
  that pin there; `main` keeps the path dep by construction. This is exactly how
  `hana` pins `bevy_kana` and `hana_lagrange`. Initial entry: `version = "0.6.0"`.
- **Consequence, accepted:** a breaking `tui_pane` change now needs two releases —
  `tui_pane` first, then the pin bumps (STEP 11 of `/release` does this
  automatically), then `cargo-port`. That decoupling is the point; it also ends
  today's constraint that `cargo-port` and `tui_pane` carry the same version number.
- `pre_release_checks.sh` stays workspace-wide, so every release still builds,
  lints, and tests all three tools before any one of them ships.

### Shared release-skill edits required

`~/.claude/commands/release.md` is shared by every project, so both edits below
must be backward compatible. Neither affects `hana` or `bevy_brp`, which release
whole-workspace.

1. **`install_verify` must follow the released package.** Today it is a scalar
   crate dir (`install_verify = "mcp"`) resolved once. With three binaries in one
   repo, a single value installs the wrong crate. Change: when
   `${SINGLE_PACKAGE_MODE}` is true, `${INSTALL_CRATE_NAME}` resolves to
   `${PACKAGE}` and the config value acts only as an on-switch. Whole-workspace
   mode is untouched.
2. **`install_verify_script` needs the crate name.** It is currently invoked as
   `${INSTALL_VERIFY_SCRIPT} ${VERSION}`. Append `${INSTALL_CRATE_NAME}` as `$2`.
   Existing scripts read only `$1` and ignore extra positional args, so
   `cargo-mend`'s current script keeps working unchanged.

`cargo-liner`'s own `install_verify.sh` then dispatches on `$2`: plain
`cargo install` for `cargo-port` and the tile tool, `RUSTC_BOOTSTRAP=1 cargo
+stable install` for `cargo-mend`, and a no-op for `tui_pane` (a library).

## Friction this plan has to handle

These are the concrete items found while reading both repos. None is a blocker;
all of them fail quietly if skipped.

| # | Item | Where | Handling |
|---|---|---|---|
| 1 | `RUSTC_BOOTSTRAP=1` becomes workspace-wide | `cargo-mend` needs it for `#![feature(rustc_private)]` in `src/main.rs`; it sets it repo-wide in `.cargo/config.toml [env]`. Cargo has no per-package env, so at the monorepo root it applies to `cargo-port` too. | `cargo-port` and `tui_pane` have no `#![feature]` attributes today, so the only risk is that one could be added and silently compile on stable. Keep the root `[env]` and add a CI job that builds `cargo-port` with `RUSTC_BOOTSTRAP` cleared. |
| 2 | `build.rs` rerun triggers break | `cargo-mend/build.rs` emits `cargo:rerun-if-changed=.git/HEAD` and `.git/refs/heads`, resolved relative to the package root. At `crates/cargo-mend/` those paths stop existing. | Rewrite to `../../.git/HEAD` and `../../.git/refs/heads`. Without this the stale `MEND_GIT_HASH` / `MEND_BUILD_ID` bug those directives were added to prevent returns, and mend silently serves cached findings from an older build. |
| 3 | `unsafe_code` and `undocumented_unsafe_blocks` | `cargo-port`'s workspace lints deny both; `cargo-mend` has neither today and has 4 `unsafe` sites in `src/main.rs`. | Add `// SAFETY:` comments and one reasoned `#[allow(unsafe_code, reason = …)]`. `missing_docs` costs nothing — `cargo-mend` is bin-only with no public API surface. |
| 4 | CI cost triples | `cargo-mend` jobs need `rust-src, rustc-dev, llvm-tools-preview`; `cargo-port` jobs need the Linux apt package set plus a Windows job. | One workflow with hana's `dorny/paths-filter` `changes` job. Shared `format`/`taplo` always run; each tool's build/test/clippy jobs gate on its own `crates/<name>/**` filter. |
| 5 | Tag collision | Both repos have `v0.6.0`, `v0.5.0`, etc. A naive graft cannot hold both. | `git filter-repo --tag-rename` during each import: `v*` → `cargo-port-v*` and `cargo-mend-v*`. This also matches what `/release` generates going forward, so `git describe --match "${PACKAGE}-v*"` finds historical tags for changelog generation. Originals stay untouched in the archived repos. |
| 6 | Release branches | `cargo-mend` has ~40 `release-0.x` remote branches, `cargo-port` ~20. | Import `main` only. They are fire-and-forget snapshots and remain reachable in the archived repos. |
| 7 | `~/.claude` tooling references the old dirs | `scripts/clean-fix/clean-fix.conf` `[build]`, `[projects]`, and `[project_env]`; `.claude/settings.local.json`; `commands/mend_fix.md` (hard-codes `/Users/natemccoy/rust/cargo-mend`). | `clean-fix.conf` already supports the `<dir>/<subpath>` member form used by every `bevy_hana/crates/*` entry, so this is a config rewrite, not a code change. Left undone, nightly clean-fix silently stops covering both tools. |
| 8 | `git-filter-repo` is not installed | — | `brew install git-filter-repo` in Phase 0. |
| 9 | Repo name collides with an unrelated crates.io crate | `cargo-liner` exists on crates.io as an unrelated tool. | No conflict in practice — `cargo-liner` is a repo name only and is never published as a crate, exactly as `hana` is a repo name and not a crate. Worth knowing it will muddy search results. |
| 10 | Scripts that derive a repo root from their own directory | `scripts/check-no-test-abort.sh` computed `SCRIPT_DIR/..`. Left at `crates/cargo-port/scripts/`, that resolved to the crate dir, so the abort-inventory gate silently stopped scanning `tui_pane` and still reported OK. | Moved to the workspace root `scripts/`, where the existing `SCRIPT_DIR/..` idiom is correct again. Phase 3 must audit `cargo-mend`'s `.github/scripts/validate-ci.sh` the same way. Found only by reading the script — it passes either way. |
| 11 | `git mv <crate>/.claude .claude` nests instead of renaming | A root `.claude/` already existed (local session settings), so git moved the directory *into* it, producing `.claude/.claude/config/release.toml`. `taplo fmt` surfaced it by listing both copies. | Move the contents, or confirm the destination is absent first. Phase 3 hits this again with `cargo-mend`'s `.claude/`, which also holds a `scripts/release/` tree. |
| 12 | README and doc path references | `LICENSE-MIT`/`LICENSE-APACHE` moved to the root, so relative links from `crates/cargo-port/README.md` broke; the CI badge and `git clone` instructions still named the old repo; `docs/tooltip.md` (an unimplemented plan) carried 20 `tui_pane/…` paths. | License links became absolute `github.com/natepiano/cargo-liner/blob/main/…` URLs, which resolve on GitHub and on crates.io alike. `tooltip.md` paths rewritten. Historical as-built and completed-plan docs left as written — they describe past work. |
| 13 | Actions is slow to register a workflow on a brand-new repo | For ~25s after the initial push, `gh run list` showed no run at all while `gh workflow list` already reported the workflow `active`. A manual `gh workflow run` filled the gap, and when the push-triggered run finally started, `cancel-in-progress` cancelled the manual one. | Wait rather than dispatch. Also note `gh run watch --exit-status` exits 0 on a **cancelled** run — it only fails on `failure`, so a cancelled run reads as success. Check `conclusion` explicitly. |
| 14 | `mktemp -t` fails under the command sandbox | `check-no-test-abort.sh` dies with `mkstemp failed … Operation not permitted`. | Run it with the sandbox disabled. It is a sandbox limit, not a defect in the script — do not "fix" the script. |
| 15 | CI installs `stable`, the toolchain moves, clippy tightens | `dtolnay/rust-toolchain@master` with `toolchain: stable` resolves to whatever stable is current. Stable 1.98.0 landed 2026-08-18, two days before the migration, while local was 1.97.0 — so CI failed on 5 findings in untouched cargo-port source that local clippy could not see: an unused `use confique::Config as _;` in a test module, four `missing_const_for_fn`, and one `.ok().is_some_and(..)` on a `Result`. | Not a migration regression — the old repo ran the identical clippy invocation. **Resolved**: `rustup update stable` to reproduce, then fix. Diagnose this class by comparing `rustc --version` against the CI log before suspecting the merge. |
| 17 | Updating the stable toolchain breaks the installed `cargo-mend` | `cargo mend` died with `dyld: Library not loaded: @rpath/librustc_driver-<hash>.dylib`. The installed binary links `rustc_private` from the exact stable it was built against, and `rustup update` deletes that library. | Rebuild it: `RUSTC_BOOTSTRAP=1 cargo install --path <cargo-mend>`, and confirm `rustc-dev` is installed for the new toolchain first. Any stable bump breaks the mend gate this way, so Phase 3 should expect it. If the cargo-mend tree has uncommitted work, build from a throwaway clone at its committed HEAD rather than the dirty tree. |
| 16 | `/release` dry-run cannot complete for a project with `[[publish_path_pins]]` | STEP 6 pins path-only deps *before* the publish dry-run, but under `--dry-run` `pin_path_deps.sh` only reports. `cargo publish --dry-run` then runs against the unpinned manifest and always fails with `dependency 'tui_pane' does not specify a version`. The step whose purpose is to make publish possible is the step dry-run skips. | A gap in `~/.claude/commands/release.md` STEP 6, not in cargo-liner — it hits `bevy_hana`'s `bevy_kana` pin identically. Cleanest fix: have `pin_path_deps.sh` apply the rewrite even in dry-run, skip only the commit, and restore `Cargo.toml`/`Cargo.lock` after the publish dry-run, so one script owns both the edit and its undo. |

## Phases

`cargo-mend` has uncommitted work in progress, so it is untouched until Phase 3.
It keeps developing in its own repo while Phases 1–2 run.

### Phase 0 — prerequisites — done 2026-08-20

No repository is modified.

1. `brew install git-filter-repo`.
2. Create the empty public repo `natepiano/cargo-liner` (no README, no license —
   the graft supplies the initial commit history).
3. Confirm `~/rust/cargo-port` has a clean working tree and `main` is pushed.

### Phase 1 — build the monorepo from cargo-port's history — done 2026-08-20

1. Fresh single-branch clone to `~/rust/cargo-liner`
   (`git clone --single-branch --branch main`; `filter-repo` requires a fresh clone
   and this drops the release branches in one move).
2. `git filter-repo --to-subdirectory-filter crates/cargo-port`, then a second pass
   `--path-rename crates/cargo-port/tui_pane/:crates/tui_pane/`, then
   `--tag-rename '':'cargo-port-'`. The rename also prefixes the two non-release
   tags (`phase-0-start`, `project-list-refactor-start`); harmless, since
   `--match "cargo-port-v*"` still selects only releases.
3. One ordinary commit lifts the shared files back to the root: `rustfmt.toml`,
   `taplo.toml`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore`, `.cargo/`,
   `.github/`, `.claude/`, `Cargo.lock`, and `scripts/check-no-test-abort.sh`
   (friction 10). Watch the `.claude/` nesting trap in friction 11. Also re-seed
   `.git/info/exclude` — it is local-only, so the graft does not carry it, and
   without it `settings.local.json` and the claude-code runtime files show up as
   untracked. Everything else stays inside `crates/cargo-port/` —
   including `assets/`, `tests/`, `scripts/`, and `docs/`, so the README image
   links and the `exclude = ["assets/"]` manifest key need no edits. Only genuinely
   cross-tool docs (this file among them) move to the root `docs/`.
4. Root virtual `Cargo.toml`: `[workspace] members = ["crates/*"]`, `resolver = "3"`,
   `[workspace.package]`, the existing `[workspace.dependencies]` with `tui_pane`
   reduced to path-only, and `[workspace.lints]` carried over verbatim.
5. `crates/cargo-port/Cargo.toml` and `crates/tui_pane/Cargo.toml`: switch
   `authors`/`edition`/`license`/`repository` to `.workspace = true`, set the
   per-crate `homepage`, keep each version as-is, keep `[lints] workspace = true`.
6. New root `README.md` indexing the crates.
7. `.github/workflows/ci.yml`: add the `changes` paths-filter job and gate the
   existing `cargo-port` jobs on it. The scaffolding goes in now so Phase 3 only
   adds a filter and its jobs.
8. `.claude/config/release.toml`: drop `workspace_publish`, add the `tui_pane`
   `[[publish_path_pins]]` entry at `0.6.0`, add `install_verify` plus the
   dispatching `install_verify.sh`.
9. Apply the two shared release-skill edits described above (**applied**). They are only strictly
   needed once a second binary lands, but doing them here means the Phase 1 dry-run
   exercises the same code path the real releases will use.
10. Gates: `cargo build --workspace`, `cargo +nightly fmt --all -- --check`,
    `taplo fmt --check`, `cargo clippy --workspace --all-targets --all-features`,
    `cargo nextest run --workspace`, `cargo mend --workspace`,
    `cargo install --path crates/cargo-port`.
11. Push `main` and the renamed tags to `natepiano/cargo-liner`; watch CI green.
12. `/release cargo-port 0.7.0 dry-run` — rehearses single-package mode, the
    `tui_pane` path pin, and `install_verify` dispatch without publishing.
    **Run 2026-08-20.** Every step behaved correctly: `release-cargo-port-0.7.0`
    for the branch, `cargo-port-v0.7.0` for the tag, `cargo-port` bumped alone
    while `tui_pane` stayed at `0.7.0-dev`, `update_workspace_deps --auto`
    correctly no-opped on the path-only entry, and all four `install_verify.sh`
    dispatch branches returned the right code. The publish step is the one that
    cannot pass in dry-run mode — see friction row 16; applying the pin by hand
    makes it pass cleanly, which also confirms current cargo-port source still
    compiles against the published `tui_pane` 0.6.0.

    One content gap, not a mechanism gap: `crates/cargo-port/CHANGELOG.md` has an
    empty `[Unreleased]`. A real 0.7.0 needs entries first — at minimum the move
    to `natepiano/cargo-liner`.

### Phase 2 — archive the cargo-port repo — done 2026-08-20

Archival needs a final commit *before* `gh repo archive`, since an archived repo
is read-only: a move notice at the top of `README.md`, and removal of the CI
badge (archiving disables Actions, so the badge would go permanently stale).
Use `[skip ci]` in that commit so the old repo's CI does not run against a
toolchain that has since moved. Old repo had 3 stars, 0 forks, and no open
issues or pull requests, so nothing needed migrating.

1. `gh repo edit natepiano/cargo-port --description "MOVED → github.com/natepiano/cargo-liner (crates/cargo-port) — …"`, then `gh repo archive`.
   Matches the `bevy_lagrange` precedent; the URL keeps resolving, so the
   `repository` link on every already-published `cargo-port` version still works.
2. Update `~/.claude` tooling for the new paths. `clean-fix.conf` `[build]`
   becomes the bare `cargo-liner` entry, which builds the whole workspace the way
   the `bevy_hana` root entry does. `[projects]` uses the member form
   `cargo-liner/crates/cargo-port`, because the history key is the entry's last
   path segment — so style-eval history carries over unbroken — plus a new
   `cargo-liner/crates/tui_pane` entry. That second entry starts a fresh style
   identity: `tui_pane` was previously covered by cargo-port's workspace-wide
   eval and now, as a peer, gets its own, matching how each `bevy_hana` member is
   listed separately.
3. Leave `~/rust/cargo-port` on disk. Removing the local checkout is a separate,
   explicitly confirmed step after the new repo has proven itself.

### Phase 3 — import cargo-mend

Prerequisite: the in-flight `cargo-mend` work is committed and pushed.

1. Fresh single-branch clone of `cargo-mend` to a scratch path;
   `git filter-repo --to-subdirectory-filter crates/cargo-mend --tag-rename '':'cargo-mend-'`.
2. In `cargo-liner`: add the scratch clone as a remote, fetch,
   `git merge --allow-unrelated-histories cargo-mend/main`, fetch its tags.
3. Manifest: `[package]` inheritance, `[lints] workspace = true`, and hoist its
   deps into `[workspace.dependencies]` — new entries for `clap`, `proc-macro2`,
   `quote`, `regex`, `rustc-hash`, `syn`, `toml_edit`; `anyhow`, `cargo_metadata`,
   `dirs`, `rayon`, `serde`, `serde_json`, `tempfile`, `toml`, and `walkdir`
   already exist at compatible versions and unify. Keep the `test-counters`
   feature and the `[[test]] name = "diagnostics"` target.
4. Friction items 1, 2, and 3 from the table: root `.cargo/config.toml` `[env]`,
   the `build.rs` `../../.git/…` paths, and the `unsafe` annotations.
5. CI: add the `cargo-mend` filter and its jobs, plus the `RUSTC_BOOTSTRAP`-cleared
   `cargo-port` guard build.
6. `install_verify.sh` gains the `cargo-mend` branch; `cargo-mend`'s own
   `.claude/scripts/release/install_verify.sh` is folded into it and deleted.
7. Gates: the full Phase 1 gate list plus `cargo install --path crates/cargo-mend`
   and `/release cargo-mend 0.19.0 dry-run`.

### Phase 4 — archive the cargo-mend repo

Same as Phase 2: `MOVED →` description, archive, then update `clean-fix.conf`
(including moving its `[project_env] cargo-mend=RUSTC_BOOTSTRAP=1` line to the new
path) and the hard-coded source path in `~/.claude/commands/mend_fix.md`.

### Phase 5 — first real releases from the monorepo

Ship `cargo-port` and `cargo-mend` once each from `cargo-liner` to confirm the
single-package pipeline end to end against crates.io before the third tool lands.

### Phase 6 — the tile tool

Scaffold the ratatui replacement for the `cargo-tile` zsh scripts
(~1760 lines across `cargo-tile`, `cargo-tile-pane`, `cargo-tile-slot`,
`cargo-tile-summary`, and the `cargo` shim) as a fourth crate. Its design is
separate work; what matters here is that it is the first consumer of `tui_pane`
other than `cargo-port`, which is the real test of promoting `tui_pane` to a peer.
Its crate name is still to be chosen.

## Open follow-ups

- Name for the tile tool crate.
- Whether the `cargo-tile` scripts stay in `~/.claude/scripts/` during Phase 6 or
  move into the new crate's repo as a reference implementation.
