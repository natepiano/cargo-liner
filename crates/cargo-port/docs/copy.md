# Focused Copy Plan

## Goal

Add framework-owned copy support for the currently focused pane row.

The framework owns the copy service, copy registration, and clipboard write. The
app may bind a key to that service and decide how to show the result, such as a
toast.

## Non-goals

- Do not read the terminal's mouse or text selection.
- Do not copy automatically when the cursor moves.
- Do not make JSON export the default copy behavior.
- Do not require every pane to support copy.
- Do not bind over native terminal copy shortcuts by default.

## User Behavior

- Add one global copy command.
- Default binding: `y`.
- Keep the binding configurable through the existing keymap.
- Leave `Ctrl-C`, `Ctrl-Shift-C`, and `Command-C` unbound by default.
- Show a short result message:
  - `Copied value`
  - `Copied path`
  - `Copied URL`
  - `Nothing to copy`
  - `Clipboard unavailable`
  - `Copy failed`

## Framework API

Add copy types in `tui_pane`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyLabel {
    Value,
    Path,
    Url,
    Row,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPayload {
    pub text: String,
    pub label: CopyLabel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopySelectionResult {
    Payload(CopyPayload),
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    Copied { label: CopyLabel },
    NothingToCopy,
    Unavailable { reason: ClipboardError },
    Failed { reason: ClipboardError },
}

pub trait CopySelection<Ctx: AppContext>: Pane<Ctx> {
    fn copy_selection(ctx: &Ctx) -> CopySelectionResult;
}
```

`Copied` means the framework wrote and flushed the clipboard escape sequence to
the terminal. OSC52 does not confirm that the terminal accepted the text into the
system clipboard.

`CopySelectionResult::Nothing` is the pane saying the current row has no useful
copy value. A pane that does not implement `CopySelection` also produces
`NothingToCopy`.

## Framework Registry

Add a copy registry to `Framework`:

```rust
type CopyResolver<Ctx> = fn(&Ctx) -> CopySelectionResult;

impl<Ctx: AppContext> Framework<Ctx> {
    pub fn register_copy_selection<P>(&mut self)
    where
        P: CopySelection<Ctx> + Pane<Ctx>;

    pub fn copy_selection<B>(
        &self,
        ctx: &Ctx,
        backend: &mut B,
    ) -> CopyOutcome
    where
        B: ClipboardBackend;
}
```

`copy_selection` resolves the focused pane id, runs the registered resolver, and
writes the returned payload through the clipboard backend.

Focus rules:

- Focused app pane with a resolver: use that pane's copy value.
- Focused app pane without a resolver: `NothingToCopy`.
- Framework overlays such as keymap and settings: `NothingToCopy`.
- Text input or key capture mode: do not copy.
- Toast focus: `NothingToCopy`.

Duplicate registration for the same pane id replaces the resolver and does not
change pane order. Add a test for this.

## Builder API

Add builder support so copy behavior is registered next to pane registration:

```rust
KeymapBuilder<Ctx, Registering>
    .register::<PackagePane>()
    .register_copy_selection::<PackagePane>();
```

Implementation details:

- Store `copy_registrations: Vec<CopyRegistration<Ctx>>` in the builder.
- Carry the registrations through the builder state transition.
- In `build_into`, call `framework.register_copy_selection` for each stored
  registration.
- Add a test that a builder-registered pane can be copied after build.

This keeps copy support a framework capability while letting `cargo-port` wire a
normal app action to it.

## Clipboard Backend

Use Crossterm OSC52 for the first implementation.

Crate setup:

- Add a `clipboard` feature in `tui_pane`.
- Enable `clipboard` by default.
- Map `clipboard` to `crossterm/osc52`.
- When the feature is disabled, the backend returns `Unavailable`.

Backend API:

```rust
pub trait ClipboardBackend {
    fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError>;
}
```

Add an OSC52 backend for normal use and a fake backend for tests. The fake
backend records attempted writes so tests can assert that copy happened, did not
happen, or failed.

Do not add native clipboard tools in the first pass. A future native backend can
be added if we need stronger confirmation than OSC52 can provide.

## Keymap

Add the app-facing copy action in `cargo-port` and route it to the framework
copy service:

- Add `Copy` to the app global action enum.
- Add default `y` binding.
- Add the keymap UI row.
- Add the status strip entry in a stable order.
- Add or migrate default keymap config if the repo uses generated defaults.
- Add tests that `Ctrl-C`, `Ctrl-Shift-C`, and `Command-C` are not default copy
  bindings.

Do not add `Command-C` parsing in this change. Terminal support for Command-key
events needs a separate compatibility pass.

## Cargo-port Pane Behavior

Each pane decides what the selected row means.

### Project List

Copy the selected project path when available.

If only a display name is available, return `Nothing`. Do not copy the display
name in the first pass because it is not useful outside the app.

### Package Details

Copy the selected field value, not the label.

Rules:

- Use a pure helper such as `copy_payload_for_package`.
- Reuse the same field ordering as rendering.
- Crates.io rows copy the full crate URL.
- Local path rows copy the path.
- Normal scalar fields copy the value.
- Empty values and placeholder values return `Nothing`.
- Lint summary rows return `Nothing` in the first pass.
- CI summary rows return `Nothing` in the first pass.

### Git Details

Copy selected label/value fields through a pure helper such as `git_copy_value`.

Rules:

- Branch fields copy the branch name.
- Remote fields copy the remote URL.
- Commit fields copy the hash.
- Path fields copy the path.
- Status fields copy the status value.
- Worktree rows copy the local path.

### CI Runs

Copy the selected run URL when the selected row has one.

Rules:

- Use a pure helper such as `copy_payload_for_ci`.
- Return `Nothing` for out-of-range rows.
- Return `Nothing` for rows without URLs.

### Lints

Return `Nothing` in the first pass.

Later, if a lint row maps to exactly one log path, it can copy that path. If it
maps to a run id or archive directory instead, decide that separately.

### Targets

Return `Nothing` in the first pass.

Later, decide whether target rows should copy target identity or a runnable
command. Derive that from the same data used by target actions.

### Finder

Return `Nothing` in the first pass.

Finder has app-owned text input and navigation behavior. Avoid copy there until
we decide whether it should copy a path, a filter value, or selected entry text.

## Implementation Phases

1. Add `tui_pane` copy types, registry, builder registration, and backend trait.
2. Add OSC52 backend behind the `clipboard` feature.
3. Add `cargo-port` copy action, default `y` binding, status entry, and keymap UI
   row.
4. Implement copy helpers for Project List, Package Details, Git Details, and CI
   Runs.
5. Add user feedback for every `CopyOutcome`.
6. Add focused tests.

## Tests

Framework tests:

- Builder registration reaches the framework after `build_into`.
- Duplicate copy registration replaces the resolver.
- Pane without a resolver returns `NothingToCopy`.
- Pane returning `Nothing` does not call the backend.
- Backend failure returns `Failed`.
- Feature-disabled backend returns `Unavailable`.
- Framework overlays and toast focus do not call the backend.
- Text input and key capture modes do not call the backend.

Keymap tests:

- Default `y` binding triggers copy.
- `Ctrl-C` is not a default copy binding.
- `Ctrl-Shift-C` is not a default copy binding.
- `Command-C` is not accepted or bound until the compatibility pass.
- Keymap UI includes the copy command.

Pane tests:

- Project List copies the selected path.
- Package Details copies a normal value.
- Package Details copies a Crates.io URL.
- Package Details returns `Nothing` for lint and CI summary rows.
- Git Details copies branch, remote URL, commit hash, path, and status values.
- Git Details copies the local path for worktree rows.
- CI Runs copies the run URL.
- CI Runs returns `Nothing` for out-of-range rows and rows without URLs.

## Remaining Product Decisions

These are the only choices that still need product input:

1. Project List: should `y` copy the selected project path?
   - Recommendation: yes.
   - Decision: approved.
2. Package Details: should Crates.io rows copy the full URL?
   - Recommendation: yes.
   - Decision: approved.
3. Git Details: should worktree rows copy the local path?
   - Recommendation: yes.
   - Decision: approved.
4. Finder: should Finder stay non-copyable for the first pass?
   - Recommendation: yes.
   - Decision: approved.
