# Plan: Progressive Detail Loading (Priority-Based Scanning)

## Problem
When cargo-port starts, it scans all projects in parallel but doesn't prioritize the currently selected project. The user sees partial details (name, path, types) but has to wait for the full scan to complete before seeing CI runs, disk usage, git info, and crates.io versions for the project they're looking at.

## Goal
When the user selects a project during scanning, immediately fetch its full details (CI, disk, git, crates.io) before other projects. If the user switches selection, prioritize the new selection. The background scan continues filling in everything else.

## Approach: On-Demand Fast-Path Fetch

### Core Idea
When a project is selected and its details aren't fully loaded, spawn a one-off targeted fetch for just that project. The main scan continues independently. Deduplication handles overlap (if the main scan already fetched it, the targeted fetch is a no-op from cache).

### Changes

#### 1. Track what's been loaded per project
Add a `HashSet<String>` to `App` called `fully_loaded` keyed by project path. A project is "fully loaded" when it has disk, CI, git, and crates.io data (or those fetches have been attempted).

#### 2. Detect selection changes during scan
In `handle_event` (or `track_selection`), when the selected project changes and `!scan_complete` and the project isn't in `fully_loaded`:
- Spawn a quick background thread that fetches:
  - Disk usage (`dir_size`)
  - CI runs (`fetch_ci_runs_cached`)
  - Git info (`GitInfo::detect`)
  - Crates.io version (`fetch_crates_io_version`)
- Send results back via the existing `BackgroundMsg` channel
- Mark the project as `fully_loaded`

#### 3. New message or reuse existing
Reuse the existing `BackgroundMsg` variants (DiskUsage, CiRuns, GitInfo, CratesIoVersion). The poll_background handler already handles all of these. No new message types needed.

#### 4. Priority thread
Create a function `spawn_priority_fetch(tx: Sender<BackgroundMsg>, project: &RustProject, ci_run_count: u32, exclude_dirs: &HashSet<String>)` that:
- Takes the project's abs_path
- Runs disk, CI, git, crates.io in sequence (not parallel — it's just one project, fast enough)
- Sends each result as it completes via the existing channel

#### 5. Debounce
Don't spawn a priority fetch on every arrow key press. Use a small delay (e.g., 100ms since last selection change) before spawning. Or simpler: only spawn when the project doesn't have disk/CI data yet — if the user is scrolling fast through loaded projects, no extra work happens.

### Files to Modify

- `src/tui/mod.rs`:
  - Add `fully_loaded: HashSet<String>` to `App`
  - Add `priority_fetch_path: Option<String>` to avoid duplicate fetches
  - In `track_selection()` or `handle_event()`, check if selected project needs priority fetch
  - Add `spawn_priority_fetch()` function

- `src/tui/scan.rs`:
  - Extract `fetch_project_details()` from the rayon closure in `spawn_streaming_scan`
  - Make it callable independently for the priority path

### What NOT to change
- The main scan continues as-is — no changes to the rayon walk
- The poll_background handler stays the same
- The rendering stays the same (it already shows whatever data is available)

### Edge Cases
- User selects a project that the main scan hasn't discovered yet (shouldn't happen — projects appear as they're found)
- Priority fetch races with the main scan for the same project — handled by dedup (HashMap insert overwrites, which is fine)
- User switches selection rapidly — only the latest selection triggers a fetch; previous in-flight fetches just add harmless extra data

## Verification
1. Start cargo-port on a large directory
2. Immediately navigate to a project
3. Its CI/disk/git should appear within ~2 seconds even while scanning continues
4. Switch to another project — same fast loading
5. After scan completes, all projects have full data
