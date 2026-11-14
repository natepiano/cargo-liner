# Footer colors and readable inks

## What it is

cargo-handler reads role and account color assignments from the Claude footer configuration.
It resolves those colors against the active theme and pane backgrounds.
Session hues, selection, word matches, and reset shades use the same readability rules where their purposes overlap.

Paths below are relative to `crates/cargo-handler/src/` unless they start with `crates/`.

| File | Responsibility |
| --- | --- |
| `footer_colors/mod.rs` | Footer parsing, readable palettes, glyph contrast, and selected inks. |
| `footer_colors/constants.rs` | Contrast targets and account color seeds. |
| `theme/mod.rs` | `SessionRowInk`, `SessionHue`, theme roles, and reset ranks. |
| `theme/files.rs` | Theme parsing and installation outcomes. |
| `theme/word_match.rs` | Word-match backgrounds and foreground resolution. |
| `agent_cell.rs` | Session hue assignment and tree ink use. |
| `render/mod.rs` | Styled tile titles and selection painting. |
| `summary/accounts.rs` | Account labels, role spans, and reset shades. |

## How it works

### Footer assignments

`FooterColorFile` reads `$HOME/.claude/config/statusline.conf`.
It retains parsed `FooterColors` and `FooterFileRevision`.
`Ticker` checks the file on completed local scans.
Only a changed modification time triggers another read.
Draw functions use the retained palette and never open the file.
A missing file supplies plain theme text.

The parser accepts `key=value`, comments, and blank lines.
`FooterColor` parses recognized names into a closed enum.
Unknown names leave the corresponding entry plain.
Account keys match the displayed label through `account.<label>`.
A new account assignment needs no Rust change.

`FooterRole` covers Showrunner, UnitDirector, AstraDesigner, and AstraReviewer.
Their keys are `role.showrunner`, `role.unit-director`, `role.astra-designer`, and `role.astra-reviewer`.
`FooterColors::hierarchy_role` resolves hierarchy tints.
The same role tint serves tile suffixes, tree roles, and accounts role spans where applicable.
Ordinary worker and subagent roles retain their current plain style.
A Showrunner subagent uses the Showrunner tint.

Basic names map to the corresponding ANSI colors.
Bright names use the bright ANSI set.
Named indexed colors use these values:

| Name | xterm index |
| --- | --- |
| orange | 208 |
| purple | 141 |
| pink | 211 |
| mauve | 139 |
| teal | 30 |
| brown | 137 |

Astra reviewer green uses its own theme role after parsing the footer assignment.
Astra mauve also uses a per-theme shade.
The theme TOML files remain the source for those appearance-specific shades.
Account blue and teal use purpose-specific seeds before readability resolution.

### Waiting activity

The `waiting` theme role supplies the bright yellow ink for the live activity word `waiting`.
The bundled dark themes use bright yellow.
The bundled light themes use RGB `(122, 122, 0)`, the brightest equal-channel yellow that meets 4.5:1 on white.
The readability resolver adjusts the ink for glyph coverage and selection grounds.
The theme loader preserves the built-in value when a custom theme omits it.
A custom theme can replace that value under `[variants.roles]`.
`Role::Waiting` draws through `SessionRowInk::Waiting`, which applies the readability rules.
Selection retains readable yellow ink.
Every word-match emphasis keeps the 4.5:1 contrast floor.
The match resolver can change the foreground to satisfy that floor.
Waiting keeps a different ink from busy, idle, and unreachable.
The job before the word keeps its existing ink.

### Grounds and glyph contrast

`FooterTint::{Footer, Management, Plain}` changes only the intended foreground.
`SessionRowInk::style()` is the draw path for fixed session-row inks.
Its variants cover text, launcher labels, program labels, activity, shell timers, accounts, and management.
`SessionRowInk::fixed_fields` and `FooterColors::session_row_inks` expose the checked palette to tests.

`InkGrounds` includes the base, unfocused pane, and focused pane backgrounds.
The palette cache checks theme `Arc` identity and the current grounds.
It recomputes readable colors when either changes.
The resolver preserves a valid ink and otherwise changes lightness along its color path.
Subdued labels use a computed readable RGB color instead of terminal DIM.

`InkCoverage::Antialiased` models foreground coverage of 208/255.
`antialiased_contrast` checks both fractional channels and byte-rounded channels.
The lower ratio determines whether an ink meets the 4.5:1 floor.
Opaque contrast alone cannot prove that a thin terminal glyph is readable.
Tests use an independent literal coverage value to check the drawn output.

### Session hues and reset shades

Each theme supplies exactly five `session_hues`.
`SessionHue::of_agent` cycles through them by root ordinal.
Descendants use a separate counter.
Adding descendants therefore cannot recolor later root titles or summary names.
Sorting happens after hue assignment.

Unselected palettes also use CIEDE2000 separation checks.
Their tests hold the measured floors for each built-in theme.
Five hues cannot satisfy every proposed spacing floor with the other row inks fixed.
The current light reset band retains measured session separation floors of 14.7 and 15.3.
Do not treat those floors as an unmeasured universal distance of 20.
The dark reset steps differ mainly by hue.
The light reset steps differ by violet lightness.
`ResetRank` remains the only reset-color source.
See [accounts](accounts.md) for rank assignment and the three steps.

### Selection and word matches

`SelectedGroundInk` resolves an ink against one selection ground.
It changes OKLCH lightness while preserving hue and chroma.
Selected rows require readable contrast, rather than the unselected palette's distance floor.
Unselected colors remain unchanged.

`word_highlight.emphasis` sets the strength of a matched word.
`clear` is the default.
`subtle` retains the C240 grounds.
`clear` doubles the theme match ground's perceptual distance from the pane ground.
`strong` uses the clear ground and adds bold to the matched cells.
Subtle and clear retain every existing cell modifier.
Unmatched cells retain their complete styles.

`WordMatchGrounds` keeps separate grounds for plain, focused, and unfocused selected rows.
The selection search retains its visibility floor and readability cap.
The theme's declared appearance sets the lightness direction.
`MatchInk::resolve` keeps an existing foreground when it meets the required covered contrast.
Otherwise it applies the smallest needed lightness change along that ink's hue.
Both match caches include the theme, the source ground, and the emphasis.
The ink palette also includes the selected word.
See [interaction](interaction.md) for token rules and painted-frame lookup.

On opaque screens, `tui_pane::tint_painted` supplies the focused interior and readout ground.
Empty interior rows receive the same ground as content rows.
Transparent screens use `Color::Reset` for that ground and show focus through the border.
Frame cells retain the framework's chrome policy.
Status bar contrast checks cover all built-in themes.

## Invariants and gotchas

1. Read footer assignments from the footer file instead of copying user values into Rust.
2. Route fixed session-row colors through `SessionRowInk`.
3. Measure antialiased contrast on every actual pane ground.
4. Keep root hue assignment independent of descendants and sorting.
5. Preserve reset rank as the only reset-color source.
6. Check selection and word-match contrast after their backgrounds apply.

A raw 4.5:1 foreground can fail after glyph coverage.
Byte rounding can also change a marginal result.
A live transparent configuration does not exercise opaque focus tint.
Tests that compare unchanged inks must fix both the grid clock and status uptime.

## Why

One footer source keeps account and role meaning consistent across tools.
Ground-aware palette caching avoids repeated color searches during drawing.
Named ink purposes let tests detect new foregrounds that would otherwise escape readability checks.
