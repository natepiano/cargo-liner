# Tooltip Infrastructure Plan

## Goal

Add reusable tooltip support to `tui_pane` so client apps can attach explanatory hover text to pane titles, field labels, table headers, status-bar slots, and other rendered regions without each app rebuilding hover tracking, placement, clipping, theme behavior, and modal suppression.

`cargo-port` will be the first integration. The ownership boundary is:

- `tui_pane` owns generic tooltip data types, per-frame region registration, blocking/occlusion, hit resolution, placement, rendering, dwell state, and theme roles.
- `cargo-port` owns domain-specific tooltip text, app-specific layer constants, and registration calls at render sites that know final screen-space rectangles.

## Current Architecture

Mouse input already exists. `src/tui/input/mod.rs` records mouse events into `app.mouse_pos`, and `src/tui/render.rs` derives row hover state with `interaction::hovered_pane_row_at`.

Hit testing is row-oriented today. `src/tui/pane/mod.rs::HoverTarget` has `PaneRow`, `Dismiss`, and `ToastCard`; `tui_pane::Viewport` stores one hovered row. That is correct for click routing and row highlighting, but too coarse for title and label tooltips.

Rendering is immediate-mode. Ratatui does not report where a `Span`, `Line`, table header, or `Block` title ended up after rendering. Tooltip-capable elements must register a screen-space `Rect` while render code still knows the area and display width.

Render order matters. `cargo-port` draws tiled panes, status bar, toasts, framework overlays, app modals, and confirm popups in sequence. Tooltip resolution must respect that top surface even when the top surface has no tooltip payload.

## Design Principles

1. Tooltips are render-time decoration, not click dispatch.
   Do not extend `HoverTarget` for the first implementation.

2. Registration is explicit.
   Do not make every `label_color()` or `title_color()` span hoverable. Call sites opt in by registering final screen-space rectangles.

3. Regions are per-frame render metadata.
   Clear before a frame, register during rendering, resolve after the top visible surfaces are known, then discard on the next frame.

4. Occlusion is separate from tooltip payload.
   A modal, toast, or popup with no tooltip still blocks lower tooltips. The registry must support payload-free blocking regions.

5. Visibility policy is explicit.
   Do not infer modal suppression from enum ordering. Resolve only against the currently visible surface set.

6. Placement uses the anchor rect.
   The renderer receives the resolved region, not only the mouse position, so it can avoid covering the label/title/status slot being explained and can later support keyboard-triggered tooltips.

7. Tooltip text is plain and bounded.
   The first implementation supports optional title, body text, wrapping, width/height caps, and clipping/truncation. Rich markdown, links, and interactive content are out of scope.

8. The first release is hover-triggered, but the model must preserve keyboard access.
   Regions carry optional stable anchor IDs so a later focused-element command can resolve the same tooltip without redesigning the public types.

## Core API Shape

Add `tui_pane/src/tooltip.rs` and re-export the stable API from `tui_pane/src/lib.rs`.

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tooltip {
    pub title: Option<Cow<'static, str>>,
    pub body: Cow<'static, str>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TooltipAnchorId(Cow<'static, str>);

impl TooltipAnchorId {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self;
    pub fn as_str(&self) -> &str;
}

impl From<&'static str> for TooltipAnchorId;
impl From<String> for TooltipAnchorId;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TooltipLayer(u16);

impl TooltipLayer {
    pub const TILED_PANE: Self = Self(100);
    pub const STATUS_BAR: Self = Self(200);
    pub const TOAST: Self = Self(300);
    pub const FRAMEWORK_OVERLAY: Self = Self(400);
    pub const APP_MODAL: Self = Self(500);
    pub const APP_TOP_START: Self = Self(10_000);

    pub const fn app(raw: u16) -> Self;
    pub const fn try_app(raw: u16) -> Option<Self>;
    pub const fn raw(self) -> u16;
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TooltipPayload {
    Tooltip(Tooltip),
    Blocker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TooltipRegion {
    pub rect: Rect,
    pub layer: TooltipLayer,
    pub anchor_id: Option<TooltipAnchorId>,
    pub payload: TooltipPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTooltip {
    pub rect: Rect,
    pub layer: TooltipLayer,
    pub anchor_id: Option<TooltipAnchorId>,
    pub tooltip: Tooltip,
}
```

`TooltipLayer` is an extensible numeric newtype, not an exhaustive enum. `tui_pane` publishes framework constants; apps define their own constants through `TooltipLayer::app(raw)` when they need app-specific surfaces such as `cargo-port` confirms. `TooltipLayer::try_app(raw)` returns `None` for framework-reserved values below `APP_TOP_START`. `TooltipLayer::app(raw)` is the const convenience constructor and must panic/assert for reserved values; tests should cover both fallible rejection and infallible-constructor panic behavior.

`TooltipPayload::Blocker` represents visible UI that absorbs tooltip lookup without displaying anything. It prevents lower pane tooltips from appearing through blank modal space, toast bodies, popup borders, or other unannotated top surfaces.

`TooltipRegion` and `TooltipPayload` are implementation details unless tests need direct registry construction. The stable client-facing API should be method-driven through `TooltipRegistry` and `TooltipSink`, not through constructing regions by hand.

## Registry and Visibility Policy

```rust
pub struct TooltipRegistry {
    regions: Vec<TooltipRegion>,
    next_sequence: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VisibleTooltipLayers {
    ranges: Vec<(TooltipLayer, TooltipLayer)>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TooltipVisibilityKey {
    ranges: Vec<(TooltipLayer, TooltipLayer)>,
}

impl VisibleTooltipLayers {
    pub fn empty() -> Self;
    pub fn only(layer: TooltipLayer) -> Self;
    pub fn base() -> Self;
    pub fn with(self, layer: TooltipLayer) -> Self;
    pub fn with_range(self, range: RangeInclusive<TooltipLayer>) -> Self;
    pub fn contains(&self, layer: TooltipLayer) -> bool;
    pub fn key(&self) -> TooltipVisibilityKey;
}

impl TooltipRegistry {
    pub const fn new() -> Self;
    pub fn clear(&mut self);
    pub fn register_tooltip(
        &mut self,
        rect: Rect,
        layer: TooltipLayer,
        anchor_id: Option<TooltipAnchorId>,
        tooltip: impl Into<Tooltip>,
    );
    pub fn register_blocker(&mut self, rect: Rect, layer: TooltipLayer);
    pub fn resolve_at(&self, pos: Position, visible: &VisibleTooltipLayers)
        -> Option<ResolvedTooltip>;
    pub fn resolve_anchor(
        &self,
        anchor_id: &TooltipAnchorId,
        visible: &VisibleTooltipLayers,
    ) -> Option<ResolvedTooltip>;
}
```

Resolution algorithm:

1. Ignore empty rects and regions whose layer is outside `visible`.
2. Keep only regions whose rect contains `pos`; a higher-layer region elsewhere must not suppress a lower tooltip at the pointer.
3. Resolve matching regions by highest visible `TooltipLayer` first.
4. Within that layer, walk latest registration to earliest registration.
5. If `pos` lands in a blocker, return `None`.
6. If `pos` lands in a tooltip region, clone the minimal matched data into an owned `ResolvedTooltip`.
7. If no visible region matches, return `None`; do not fall back to lower hidden surfaces.

`resolve_anchor` uses the same visible-layer and highest-layer rules, but starts by matching tooltip regions by `TooltipAnchorId` instead of pointer position. Once the anchor region is selected, higher visible blockers whose rect intersects the resolved anchor rect suppress that anchor tooltip. Anchor IDs must be unique per visible tooltip region within a frame. Debug builds and tests should detect duplicate visible anchors; release behavior should use highest-layer/latest-registration ordering so the result is deterministic.

`VisibleTooltipLayers` stores normalized, merged inclusive ranges sorted by start layer. Equivalent construction paths must produce equal `TooltipVisibilityKey` values so dwell does not reset when policy identity is unchanged. Any real policy change must change the key so dwell restarts before showing a tooltip under the new surface policy.

`VisibleTooltipLayers` is intentionally not a maximum layer. `cargo-port` maps its active top-level surface to an explicit policy:

```rust
let visible_tooltip_layers = if app.confirm().is_some() {
    VisibleTooltipLayers::only(cargo_port_tooltips::CONFIRM_LAYER)
} else if app.overlays.is_finder_open() || app.overlays.is_sccache_open() {
    VisibleTooltipLayers::only(TooltipLayer::APP_MODAL)
} else if app.framework.overlay().is_some() {
    VisibleTooltipLayers::only(TooltipLayer::FRAMEWORK_OVERLAY)
} else {
    VisibleTooltipLayers::base()
        .with(TooltipLayer::TILED_PANE)
        .with(TooltipLayer::STATUS_BAR)
        .with(TooltipLayer::TOAST)
};
```

Framework and modal renderers must register a blocker for their full visible surface even before any tooltip text is added to that surface.

## Hover Trigger State

Do not render tooltips immediately for every mouse position. Add library state for dwell and suppression:

```rust
pub struct TooltipHoverState {
    candidate: Option<TooltipCandidate>,
    dwell: Duration,
    suspended_until_pointer_move: bool,
}

pub struct TooltipCandidate {
    key: TooltipCandidateKey,
    entered_at: Instant,
    last_pos: Position,
}

pub enum TooltipTrigger {
    Hover { pointer: Position },
    Focus,
}

impl TooltipHoverState {
    pub fn new(dwell: Duration) -> Self;
}

impl Default for TooltipHoverState;
```

`TooltipCandidateKey` should include the current layer, rect, `TooltipVisibilityKey`, and tooltip payload identity. Payload identity must include title/body content, not only anchor ID or region geometry. It may include `anchor_id` when present, but anchor ID alone is not enough because a stable ID can move to a new rect or new tooltip text after layout or async data changes.

Dynamic, scrollable, or data-dependent regions must provide `TooltipAnchorId`; the `(layer, rect)` fallback is only for static chrome. If the same screen rect resolves with a different anchor or tooltip payload, dwell resets.

Library behavior:

- Show only after the same candidate has stayed under a stationary pointer for the dwell duration.
- Stationary means no cell change by default. A future implementation may allow a small tolerance, but the first implementation should restart dwell whenever `MouseEventKind::Moved` changes `Position`, even if the pointer remains inside the same wide region.
- Reset on region change, pointer move outside the region, click, scroll, drag, keyboard input, focus lost, resize, or explicit app request.
- Reset and suspend on focus gained, focus lost, and resize until a real mouse movement event arrives. A synthetic focus-gained restoration from `last_mouse_pos` must not start tooltip dwell.
- Default dwell should be conservative, for example 600 ms.
- Tests can set dwell to zero.
- `resolve_hover_tooltip` must not clear suspension by itself. Only an input-side pointer movement notification may resume dwell.

`cargo-port` integration must clear or reset tooltip hover state on `Event::Resize`, `Event::FocusLost`, and `Event::FocusGained`; current or restored mouse coordinates can otherwise resolve against a new layout after resize or focus restoration. The input handler must call `Framework::record_tooltip_pointer_move(pos)` only from a real `MouseEventKind::Moved` event.

## Rendering and Placement

```rust
pub struct TooltipRenderOptions {
    pub max_width: u16,
    pub max_height: u16,
    pub gap: u16,
}

pub const DEFAULT_TOOLTIP_OPTIONS: TooltipRenderOptions = TooltipRenderOptions {
    max_width: 48,
    max_height: 8,
    gap: 1,
};

pub fn render_tooltip(
    frame: &mut Frame<'_>,
    resolved: &ResolvedTooltip,
    trigger: TooltipTrigger,
    options: TooltipRenderOptions,
);
```

Rendering uses a measured layout step:

```rust
pub struct TooltipLayout {
    pub outer: Rect,
    pub inner: Rect,
    pub lines: Vec<Line<'static>>,
}

pub fn measure_tooltip(
    frame_area: Rect,
    anchor: Rect,
    trigger: TooltipTrigger,
    tooltip: &Tooltip,
    options: TooltipRenderOptions,
) -> Option<TooltipLayout>;
```

Measurement contract:

- `max_width` and `max_height` are outer dimensions including borders.
- Return `None` when the frame cannot fit a bordered `3x3` tooltip.
- Clamp the measured outer width and height to `frame_area` before computing inner dimensions and wrapped lines.
- Inner content width is `outer.width - 2`; inner height is `outer.height - 2`.
- Wrap body text on Unicode display width.
- Long unbreakable words are truncated to inner width with a single-character ellipsis when space allows.
- If a title is present, reserve one inner row for it before body rows.
- If body rows exceed available height, truncate and mark the last visible row with an ellipsis.

Placement contract:

1. Prefer near the pointer for hover, but never overlap the anchor rect when a non-overlapping placement fits.
2. For focus-triggered tooltips, place relative to the anchor rect.
3. Try below-right, above-right, below-left, above-left.
4. Clamp to frame bounds only after non-overlapping placements fail.
5. Use `Clear`, a bordered `Block`, themed title/body styles, and a plain `Paragraph`.

## Theme Additions

Add a dedicated theme group:

```rust
pub struct TooltipTheme {
    pub border: StyleSpec,
    pub title: StyleSpec,
    pub body: StyleSpec,
}

pub struct Theme {
    ...
    pub tooltip: TooltipTheme,
}
```

Built-in defaults:

- Dark: border `DarkGray`, title `Yellow` bold, body `White`.
- Light: border medium gray, title dark amber bold, body `Black`.
- High-contrast dark: border `White`, title `LightYellow` bold, body `White`.
- High-contrast light: border `Black`, title dark amber bold, body `Black`.

Theme compatibility is required:

- Runtime `Theme` should have a non-optional `tooltip`.
- Theme-file parsing should accept old schema-1 theme files without a `[tooltip]` group.
- Implement compatibility with `Option<TooltipTheme>` on the file input type or serde defaults in the file-layer type, then fill appearance-appropriate defaults in `into_theme`.
- Add a regression test that parses a pre-tooltip schema-1 custom theme and fills tooltip defaults.
- Update `tui_pane/themes/*.toml` templates and keep the round-trip tests.

Accessors:

```rust
pub fn tooltip_border_color() -> Color;
pub fn tooltip_title_style() -> Style;
pub fn tooltip_body_style() -> Style;
```

## Framework Storage and Sink

`Renderable::render` receives `&Ctx`, so render-time registration needs controlled interior mutability. Choose this model directly; do not keep a competing plain `&mut TooltipRegistry` plan.

The sink must be an owned handle, not a value borrowing `&Framework`. `cargo-port` renders framework-owned surfaces such as settings, keymap, global shortcuts, and toasts while also registering tooltip blockers. A sink tied to an immutable framework borrow would conflict with those mutable field borrows. Use an owned internal registry handle so `tooltip_sink(...)` borrows `Framework` only for the duration of handle creation.

Store tooltip state in `tui_pane::Framework<Ctx>`:

```rust
pub struct Framework<Ctx: AppContext> {
    ...
    tooltip_registry: TooltipRegistryHandle,
    tooltip_hover: TooltipHoverState,
}

#[derive(Clone)]
struct TooltipRegistryHandle(Rc<RefCell<TooltipRegistry>>);
```

Expose narrow methods:

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    pub fn clear_tooltips(&self);
    pub fn tooltip_sink(&self, layer: TooltipLayer) -> TooltipSink;
    pub fn disabled_tooltip_sink(&self) -> TooltipSink;
    pub fn register_tooltip_blocker(&self, rect: Rect, layer: TooltipLayer);
    pub fn lookup_tooltip_at(
        &self,
        pos: Position,
        visible: &VisibleTooltipLayers,
    ) -> Option<ResolvedTooltip>;
    pub fn lookup_tooltip_anchor(
        &self,
        anchor_id: &TooltipAnchorId,
        visible: &VisibleTooltipLayers,
    ) -> Option<ResolvedTooltip>;
    pub fn resolve_hover_tooltip(
        &mut self,
        pos: Position,
        visible: &VisibleTooltipLayers,
        now: Instant,
    ) -> Option<ResolvedTooltip>;
    pub fn resolve_anchor_tooltip(
        &self,
        anchor_id: &TooltipAnchorId,
        visible: &VisibleTooltipLayers,
    ) -> Option<ResolvedTooltip>;
    pub fn reset_tooltip_hover(&mut self);
    pub fn suspend_tooltips_until_pointer_move(&mut self);
    pub fn record_tooltip_pointer_move(&mut self, pos: Position);
    pub fn set_tooltip_dwell_for_test(&mut self, dwell: Duration);
}
```

Do not expose both raw `&mut TooltipRegistry` access and active `RefCell` sinks during render. If tests need direct registry control, construct `TooltipRegistry` directly outside `Framework`.

The sink must encode disabled vs active:

```rust
#[derive(Clone, Debug)]
pub struct TooltipSink {
    inner: TooltipSinkInner,
}

enum TooltipSinkInner {
    Disabled,
    Active {
        registry: TooltipRegistryHandle,
        layer: TooltipLayer,
    },
}

impl TooltipSink {
    pub const fn disabled() -> Self;
    pub fn with_layer(self, layer: TooltipLayer) -> Self;
    pub fn register_rect(
        &self,
        rect: Rect,
        anchor_id: Option<TooltipAnchorId>,
        tooltip: impl Into<Tooltip>,
    );
    pub fn register_blocker(&self, rect: Rect);
}
```

`TooltipSinkInner` stays private so `Rc<RefCell<_>>` is not part of the stable public construction API. `register_rect` should call `tooltip.into()` before taking `borrow_mut()` and should hold the `RefCell` guard only for the registry mutation. `register_rect` and `register_blocker` no-op for `Disabled`.

`lookup_tooltip_at` and `lookup_tooltip_anchor` borrow the registry only long enough to clone an owned `ResolvedTooltip`. `resolve_hover_tooltip` then mutates dwell state after the registry borrow has ended. This avoids returning references tied to `RefCell` guards and makes borrow-panic risks local and testable.

Add a compile-level acceptance test or fixture render path that creates tiled-pane, status-bar, settings-overlay, keymap-overlay, global-shortcuts-overlay, and toast sinks in the same frame while mutably rendering the corresponding framework-owned surfaces.

## cargo-port Render Lifecycle

1. At the start of `src/tui/render.rs::ui`, call `app.framework.clear_tooltips()`.
2. Create owned sinks from `app.framework.tooltip_sink(layer)` before mutably rendering framework-owned surfaces.
3. Pass a tiled-pane sink through `PaneRenderCtx`.
4. Pass explicit overlay/modal/confirm sinks to render paths that do not use `PaneRenderCtx`.
5. Each visible overlay/popup registers a blocker for its full outer surface. For existing `PopupFrame` users, that means `render_with_areas().outer`, not the inner content rect. Tooltip payloads on those surfaces can come later.
6. After all surfaces render, build `VisibleTooltipLayers` from the active surface state.
7. Resolve `app.mouse_pos` through `Framework::resolve_hover_tooltip`.
8. Render only if dwell state allows it.
9. Run `sync_hovered_pane_row(app)` separately for existing row hover behavior.

Render paths needing sink coverage:

- Tiled panes through `PaneRenderCtx`.
- Settings overlay through existing `PaneRenderCtx`.
- Keymap overlay render methods.
- Global Shortcuts overlay render method.
- Finder popup manual `PaneRenderCtx`.
- Sccache popup renderer.
- Confirm popup renderer.
- Toast renderer. Active toast card blockers are mandatory even before toast tooltip payloads exist.
- Status bar renderer after final slot layout.

If a phase does not yet add tooltip text to a modal, it still registers a blocker so lower tooltips do not leak through.

## Title Registration

Pane titles are the first reusable helper.

Keep `PaneChrome::block(title, focused)` unchanged. Add helpers that compute geometry and optionally register. Grouped title helpers must operate on structured title inputs, not by parsing the final rendered string.

```rust
pub struct PaneTitleSpec<'a> {
    pub title: Cow<'a, str>,
    pub separator: PaneTitleSeparator,
    pub count: PaneTitleCount<'a>,
}

pub enum PaneTitleSeparator {
    Space,
    Colon,
}

pub struct RenderedPaneTitle {
    pub text: String,
    pub layout: PaneTitleLayout,
}

pub struct PaneTitleSegment {
    pub kind: PaneTitleSegmentKind,
    pub text: String,
    pub rect: Rect,
}

pub enum PaneTitleSegmentKind {
    BaseTitle,
    Count,
    Group { label: Cow<'static, str> },
}

pub struct PaneTitleLayout {
    pub full_rect: Rect,
    pub segments: Vec<PaneTitleSegment>,
}

pub fn render_pane_title(area: Rect, spec: &PaneTitleSpec<'_>) -> RenderedPaneTitle;
pub fn title_rect(area: Rect, title: &str) -> Option<Rect>;
```

For simple left-aligned titles, the full title starts at `area.x + 1`, `area.y` and clamps to `area.width.saturating_sub(2)`. Return `None` for empty or fully clipped titles.

Grouped title counts need segment geometry. `render_pane_title` is the single owner of both the rendered title string and segment rects for `PaneTitleCount::Grouped`, including prefixed/colon titles, Unicode width, and clipping. This is required before adding separate `Binary`, `Examples`, and `Benches` title-group tooltips.

Add a non-breaking chrome helper:

```rust
impl PaneChrome {
    pub fn block_with_title_tooltip(
        self,
        area: Rect,
        title: String,
        focused: bool,
        sink: TooltipSink,
        anchor_id: Option<TooltipAnchorId>,
        tooltip: impl Into<Tooltip>,
    ) -> Block<'static>;
}
```

## Label and Header Registration

Labels depend on local layout. Provide geometry helpers, not magic spans:

```rust
pub fn inline_rect(
    visible_area: Rect,
    x_offset: u16,
    y_offset: u16,
    text: &str,
) -> Option<Rect>;

pub fn register_label_tooltip(
    sink: TooltipSink,
    visible_area: Rect,
    x_offset: u16,
    y_offset: u16,
    label: &str,
    anchor_id: Option<TooltipAnchorId>,
    tooltip: impl Into<Tooltip>,
);
```

Rules:

- Use `unicode_width::UnicodeWidthStr`.
- Intersect with `visible_area`.
- Return/register nothing for empty intersections.
- Call sites must pass screen-space, scroll-adjusted coordinates.
- Tests cover offsets past width, partial clipping, zero-width text, and scrolled content.

For table headers, add a helper only after the table renderer owns final column x positions:

```rust
pub fn register_table_header_tooltips(
    sink: TooltipSink,
    header_area: Rect,
    columns: &[TooltipHeaderColumn<'_>],
);
```

```rust
pub struct TooltipHeaderColumn<'a> {
    pub rect: Rect,
    pub label: Cow<'a, str>,
    pub anchor_id: Option<TooltipAnchorId>,
    pub tooltip: Tooltip,
}
```

`TooltipHeaderColumn::rect` is the final screen-space visible column rect after horizontal scroll and clipping. Hidden columns and zero-width intersections must not register.

## Status Bar Integration

Status-bar geometry belongs to `tui_pane::bar`, not `cargo-port`.

Pre-layout slot sources may provide tooltip payloads or IDs, but final rects must be attached after left/center/right placement and clipping inside the status-line renderer. Slot identity must survive until that registration point.

Chosen implementation:

1. `RenderedSlot` carries spans, display width, action/slot identity, and an optional stable `TooltipDescriptor`.
2. `StatusLineGlobal` and app-provided status entries convert into `RenderedSlot` before left/center/right placement.
3. `status_line::render` accepts a `TooltipSink`.
4. `render_sections` works from structured `RenderedSlot` records, registers final clipped slot rects while painting, and only then flattens spans into the buffer.
5. Cargo-port supplies tooltip text through existing action/slot identity before the slot is flattened into spans.

Add tests for the framework `pane` slot, centered pane actions, right-side globals such as `shortcuts`, and clipped slots.

## First cargo-port Coverage

Prioritize UI elements that decode compact or non-obvious interface state:

1. Project Tree status headers/legend targets for lint, CI, git/sync, disk, and target indicators.
2. Targets title count groups: Binary, Examples, Benches, and the Running subpane title.
3. Pane titles only where the title adds real context:
   - Project Tree: selected row drives detail panes.
   - Targets: runnable Cargo targets plus live Running outline.
   - Output: captured output and selection/yank behavior.
4. Package/Git labels that are not self-evident:
   - Target dir, Manifest, Features, Upstream, Pull request, CI source.
5. Current status-bar slots `pane` and `shortcuts`, delivered with the status-bar phase before the first user-visible tooltip release is considered complete. If future `quit` or `keymap` slots become visible status-line slots, they can use the same descriptor path.

Avoid first-pass tooltips that only restate visible text. Leave dense row cells, fast-changing values, and per-icon row tooltips for a later phase unless a header/legend can explain them once.

## cargo-port Tooltip Copy Contract

Tooltip text should explain meaning or consequence first. It should not merely expand the visible label.

Rules:

- Keep body text to one or two wrapped lines at the default width.
- Lead with what the UI element means or what action it supports.
- Include current state only when it changes the user's next action.
- Avoid implementation names unless the UI already shows them.
- Avoid restating the visible label as the first words of the body.
- Use anchor-kind-aware wording. Header and legend tooltips should say "each row", "the marker", or "the column"; row-cell tooltips may say "this row".

Examples:

- Project Tree title: "Selecting a row drives the Package, Git, Targets, Lint, and CI panes."
- Lint status header: "Shows the most recent lint result for each row; running state also appears as a toast."
- CI status header: "Shows GitHub runs for the branch-owning row, when cargo-port can identify a repository."
- Target dir: "Cargo output directory this project will clean or inspect."
- Upstream: "Remote branch used to compute ahead/behind and sync status."
- Running title: "Live cargo-launched processes grouped by target and parent process."
- Status `pane`: "Move focus between visible panes."

Copy inventory:

- Own first-pass copy in `src/tui/tooltips.rs`, keyed by stable `TooltipAnchorId` or by a small enum that converts to the ID.
- Each inventory entry stores surface kind, visible label, optional title, body, and default layer.
- Render sites should call inventory helpers instead of inlining body strings.
- Tests must assert all first-pass anchors have inventory entries, each body is nonempty, no body starts by restating the visible label, and each body wraps within the default tooltip width without exceeding the default height.

## Implementation Phases

### Phase 1: Library Model, Registry, Dwell, Renderer

Files:

- `tui_pane/src/tooltip.rs`
- `tui_pane/src/lib.rs`
- `tui_pane/src/theme/mod.rs`
- `tui_pane/src/theme/accessors.rs`
- `tui_pane/src/theme/builtins.rs`
- `tui_pane/themes/*.toml`
- `tui_pane/tests/themes.rs`
- new `tui_pane/tests/tooltips.rs`

Work:

1. Add tooltip data types, extensible layer newtype, payload/blocker regions, registry, visible-layer policy, hover state, and renderer.
2. Implement `measure_tooltip`.
3. Add theme group with backward-compatible theme-file parsing.
4. Add direct registry and renderer tests.

Tests:

- `registry_resolves_last_registered_tooltip_in_same_layer`
- `registry_prefers_later_registered_region`
- `higher_layer_precedes_later_lower_layer`
- `higher_layer_blocker_occludes_later_lower_tooltip`
- `higher_layer_blocker_elsewhere_does_not_occlude_lower_tooltip`
- `blocker_occludes_lower_tooltip`
- `visible_policy_does_not_fall_back_to_hidden_lower_layer`
- `equivalent_visible_policy_keeps_same_visibility_key`
- `changed_visible_policy_resets_hover_dwell`
- `public_app_layer_constructor_rejects_reserved_range`
- `resolve_anchor_respects_visible_layers`
- `resolve_anchor_is_occluded_by_intersecting_higher_blocker`
- `duplicate_visible_anchor_is_detected_in_debug_or_tests`
- `hover_dwell_blocks_immediate_render`
- `hover_state_resets_on_region_change`
- `hover_state_resets_on_stationary_cell_change_inside_same_region`
- `hover_state_resets_when_tooltip_payload_changes_at_same_rect`
- `hover_state_resets_on_click_scroll_drag_key_resize_focus_lost`
- `hover_suspension_resumes_only_on_real_mouse_moved_event`
- `placement_uses_anchor_and_avoids_overlap_when_possible`
- `placement_shifts_left_on_right_edge`
- `placement_flips_above_on_bottom_edge`
- `measure_returns_none_below_3x3`
- `measure_clamps_outer_size_before_wrapping`
- `long_word_truncates_to_inner_width`
- `old_schema_theme_without_tooltip_loads_with_defaults`
- theme template round-trip tests still pass.

### Phase 2: Framework Storage and cargo-port Lifecycle

Files:

- `tui_pane/src/framework/mod.rs`
- `src/tui/render.rs`
- `src/tui/input/mod.rs`
- `src/tui/app/mod.rs`
- `src/tui/app/tests/interaction.rs`

Work:

1. Store `TooltipRegistryHandle` and `TooltipHoverState` in `Framework`.
2. Expose narrow owned-sink, clear, lookup, resolve, reset, pointer-move, and test-dwell methods.
3. Clear registry at frame start.
4. Reset hover state on key, click, scroll, drag, resize, focus gained, focus lost, and explicit modal changes.
5. Suspend dwell until a real mouse movement after resize or focus restoration.
6. Register blockers for toasts, overlays, modals, and confirm surfaces.
7. Use full outer popup/card rects for blockers, not inner content rects.
8. Resolve and render as the last visual pass.

Tests:

- base tiled tooltip appears after dwell.
- Settings, Keymap, Finder, Sccache, and Confirm block tiled tooltips on blank modal space.
- active toast cards block tiled tooltips over their full card, body, and close-button area.
- popup blockers cover the outer rect including border, title row, and cleared padding.
- modal-registered tooltip can resolve while that modal is active.
- owned tooltip sinks can be created before mutably rendering framework-owned panes and toasts in the same frame.
- stale mouse position after resize does not show a tooltip until fresh mouse movement.
- FocusGained restoration does not start dwell until a real mouse movement event arrives.
- periodic redraw using restored `last_mouse_pos` does not resume suspended dwell.
- hidden bottom-row pane registers no tooltip.
- final `TestBackend` buffer tests cover visible/hidden tooltip render order.

### Phase 3: Title Helpers

Files:

- `tui_pane/src/pane/chrome.rs`
- `tui_pane/src/pane/title.rs`
- `tui_pane/src/pane/mod.rs`
- `tui_pane/tests/tooltips.rs`

Work:

1. Add simple title rect and title layout helpers.
2. Add grouped title segment layout for `PaneTitleCount::Grouped`.
3. Add `PaneChrome::block_with_title_tooltip`.
4. Keep existing `PaneChrome::block` behavior unchanged.

Tests:

- simple title rect starts at `area.x + 1`, `area.y`.
- rect clamps to top-border width and returns `None` when fully clipped.
- grouped title layout emits segment rects for each group label.
- prefixed grouped title layout emits correct segment rects without parsing the final string.
- chrome helper registers exactly once and returns normal chrome.

### Phase 4: cargo-port Title and Legend Rollout

Files:

- `src/tui/pane/mod.rs`
- `src/tui/panes/pane_impls.rs`
- `src/tui/panes/project_list.rs`
- `src/tui/panes/package.rs`
- `src/tui/panes/git.rs`
- `src/tui/panes/targets/mod.rs`
- `src/tui/panes/targets/running_subpane.rs`
- `src/tui/render.rs`
- new `src/tui/tooltips.rs`
- focused tests under `src/tui/app/tests/interaction.rs`

Work:

1. Extend `PaneRenderCtx` with `TooltipSink`.
2. Add `src/tui/tooltips.rs` for domain text and stable anchor IDs.
3. Register title, grouped title, and compact legend/header tooltips from the first coverage list.
4. Keep row-cell tooltips out of this phase unless their header/legend cannot explain the state.

Tests:

- each selected first-pass target registers expected anchor IDs.
- no tooltip registers for hidden bottom-row pane.
- header/legend hitboxes align with visible columns.
- same rect with a different dynamic anchor resets dwell.
- same rect and dynamic anchor with different tooltip title/body resets dwell.
- tooltip text does not require selected project data unless explicitly selection-specific.
- first-pass tooltip inventory covers every expected anchor ID.
- tooltip bodies are nonempty, do not begin by restating the visible label, and fit default wrapping limits.

### Phase 5: Status Bar

Files:

- `tui_pane/src/bar/status_line.rs`
- `tui_pane/src/bar/status_bar.rs`
- `tui_pane/src/bar/slot.rs`
- `tui_pane/src/bar/tests.rs`
- `src/tui/render.rs`

Work:

1. Carry optional tooltip descriptor on structured `RenderedSlot` records.
2. Preserve slot identity through left/center/right placement and clipping.
3. Register final slot rects after status-line placement and clipping, then flatten spans into the buffer.
4. Add cargo-port slot tooltip text through existing action/slot identity.

Tests:

- `pane`, centered pane-action, and right-global slot rects match final rendered cells.
- clipped slots do not register invisible cells.
- status-bar tooltip resolution works after status-bar render.

### Phase 6: Documentation and Closeout

Files:

- `README.md` if user-facing behavior deserves mention.
- `CHANGELOG.md`
- `docs/tooltip.md` if implementation changes the plan.

Work:

1. Add a short changelog entry under `[Unreleased] > Added`.
2. Document only user-visible behavior.
3. Run validation.

Validation:

```sh
cargo +nightly fmt --all
cargo nextest run -p tui_pane
cargo nextest run -p cargo-port tui::app::tests::interaction
cargo nextest run -p cargo-port
```

For non-doc implementation closeout, also run:

```sh
cargo mend --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features
git diff --check
cargo nextest run --workspace
cargo install --path .
```

## Failure Modes and Mitigations

- **Lower tooltip leaks through modal:** visible-layer policy plus full-surface blockers.
- **Higher-layer surface loses to later lower registration:** resolve by highest layer first, latest registration only within a layer.
- **Higher-layer surface elsewhere suppresses lower tooltip:** filter to regions that contain the pointer before layer ordering.
- **Anchor tooltip appears under modal/toast:** suppress anchor lookup when a higher visible blocker intersects the resolved anchor rect.
- **Tooltip leaks through popup border:** register blocker from full outer popup/card rects.
- **Tooltip appears instantly during interaction:** dwell state plus reset on click, scroll, drag, key, resize, and focus changes.
- **Tooltip appears while pointer moves inside a wide region:** dwell requires stationary pointer cells, not only same-region hover.
- **Stale regions after resize or hidden panes:** clear registry every frame and suspend tooltip dwell until real pointer movement after resize/focus changes.
- **Tooltip fights row hover:** keep tooltip hover state separate from `Viewport::hovered`.
- **Borrowing pressure in render split:** use owned disabled/active `TooltipSink` handles backed by a private registry handle; do not keep a framework borrow alive while rendering framework-owned panes.
- **RefCell borrow leaks from lookup into render:** clone an owned `ResolvedTooltip` while the registry borrow is short-lived, then mutate hover state after the borrow ends.
- **Equivalent visibility policies reset dwell:** normalize `VisibleTooltipLayers` and compare `TooltipVisibilityKey`.
- **Incorrect title hitbox:** centralize title layout helpers and test them.
- **Grouped title tooltips duplicate formatting math:** make grouped title geometry part of the title builder before registering per-group tooltips.
- **Label rect fires for clipped text:** helpers return `Option<Rect>` after visible-area intersection.
- **Status-bar rects drift or lose identity:** carry structured `RenderedSlot` records through placement, then register after final clipping.
- **Tiny terminal overflow:** `measure_tooltip` owns all outer/inner dimensions and can skip below `3x3`.
- **Theme schema breakage:** accept old schema-1 files and fill tooltip defaults.
- **Too much tooltip noise:** first pass favors headers/legends and compact state explanations over obvious labels.

## Acceptance Criteria

- `tui_pane` exposes reusable tooltip data types, registry, visible-layer policy, dwell state, sink, and renderer.
- Tooltips and blockers can be registered from framework-owned surfaces and app-owned panes in one frame registry.
- Resolution honors top-surface occlusion at the pointer or anchor rect and never falls back to hidden lower layers.
- Hover suspension resumes only from real pointer movement, not from redraws using restored mouse coordinates.
- Placement never draws outside the terminal frame and avoids the anchor when possible.
- Title and label helpers do not change existing rendering for non-opt-in panes.
- Status-bar tooltips preserve slot identity through layout and clipping.
- Existing custom theme files without tooltip fields still load with defaults.
- `cargo-port` has visible hover tooltips on the first selected high-value titles, legends, headers, and labels.
- `cargo-port` tooltip copy is centralized in an inventory and covered by completeness/quality tests.
- Tests cover registry resolution, blockers, dwell/reset behavior, placement, theme compatibility, title rect math, status-bar rects, and cargo-port render-order integration.
- Validation uses `cargo +nightly fmt --all` and `cargo nextest run` as required for this repo.

## Team Review Record

### Cycle 1

Recorded refinements:

- Replaced `layer <= max_layer` with explicit visible-layer policy.
- Added payload-free blockers for top-surface occlusion.
- Made `TooltipLayer` an extensible newtype with app-reserved values instead of a closed enum containing cargo-port-specific `Confirm`.
- Chose `RefCell<TooltipRegistry>` plus disabled/active `TooltipSink` to match `Renderable::render(&Ctx)`.
- Added dwell state and reset requirements for click, scroll, drag, key, resize, and focus changes.
- Changed rendering API to use `ResolvedTooltip` and anchor-aware placement.
- Added stable optional `TooltipAnchorId` for future keyboard-triggered tooltips.
- Required backward-compatible parsing for old theme files without tooltip groups.
- Required status-bar rect registration after final status-line layout.
- Required title segment geometry before per-group title tooltips.
- Required visible-area clipping for label/header helpers.
- Added final render-order buffer tests, not only registry-level assertions.

Cycle 1/3: 12 refinements recorded, 0 proposed user decisions.

### Cycle 2

Recorded refinements:

- Added public constructors/accessors for `TooltipLayer`, `TooltipAnchorId`, and `VisibleTooltipLayers`.
- Kept `TooltipSink` opaque and method-driven so `RefCell` does not leak into the stable public API.
- Required `Tooltip`, `TooltipPayload`, and `ResolvedTooltip` to be cloneable enough for owned lookup results.
- Split raw registry lookup from dwell-gated hover resolution; hover resolution mutates framework state after the registry borrow ends.
- Added anchor-based lookup for future keyboard-triggered tooltips.
- Required anchor IDs to be unique per visible frame and mandatory for dynamic/data-dependent regions.
- Changed resolution to highest visible layer first, then latest registration within that layer.
- Made active toast blockers mandatory.
- Required popup/modal blockers to use full outer rects.
- Added stationary-pointer dwell and suspension until real pointer movement after resize/focus restoration.
- Required grouped title layout to be built from structured title inputs, not parsed rendered strings.
- Chose status-line render-time registration as the status-bar tooltip carrier.
- Added a cargo-port tooltip copy contract and examples.

Cycle 2/3: 13 refinements recorded, 0 proposed user decisions.

### Cycle 3

Recorded refinements:

- Chose owned `TooltipSink` handles backed by a private registry handle to avoid `&Framework` borrow conflicts while rendering framework-owned panes and toasts.
- Added an explicit `record_tooltip_pointer_move` API so resize/focus suspension resumes only from real `MouseEventKind::Moved` input.
- Added `TooltipHoverState::new`, `Default`, and a framework test-dwell hook so dwell behavior is constructible and testable.
- Specified `TooltipLayer::try_app` plus `TooltipLayer::app` panic/assert behavior for reserved framework-layer values.
- Added normalized `TooltipVisibilityKey` semantics and tests for equivalent-policy stability and real-policy reset.
- Clarified pointer lookup so higher-layer regions outside the pointer do not suppress lower tooltips.
- Defined anchor lookup occlusion by higher visible blockers intersecting the resolved anchor rect.
- Required changed tooltip title/body text to reset dwell even when layer, rect, and anchor are unchanged.
- Chose structured `RenderedSlot` records as the status-line tooltip carrier through placement and clipping.
- Added status-line tests for `pane`, centered pane actions, right-side globals, and clipped slots.
- Defined `TooltipHeaderColumn` with final screen-space clipped rects.
- Corrected first-pass cargo-port Targets coverage to current title groups: `Binary`, `Examples`, `Benches`, and `Running`.
- Corrected first-pass status-bar coverage to current visible slots: `pane` and `shortcuts`.
- Added a centralized cargo-port tooltip copy inventory with completeness, nonempty-body, non-restating, and wrapping tests.

Cycle 3/3: 14 refinements recorded, 0 proposed user decisions.

## Proposed user decisions

No unresolved decisions. Review findings that converged to one implementation path are recorded above and folded into the plan.
