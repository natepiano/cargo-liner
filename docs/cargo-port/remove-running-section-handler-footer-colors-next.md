# remove-running-section-handler-footer-colors-next

Approved next items from `remove-running-section-handler-footer-colors.md`. Each item becomes a phase when the showrunner places it.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Items

### Session colours stay apart

- **Source:** phase 26, 2026-10-23. Approved by the showrunner 2026-10-23 19:30 AST.
- **Need:** on Default Dark and High Contrast Dark, hue alone cannot separate five session inks from the fixed inks that a session row draws (`SessionRowInk`). Phase 26 measured it: the valid hue domain is about 92.5°, and at most 2–3 inks can be CIEDE2000 >= 20 apart; steps of 45°, OKLCH C >= 0.10, and both together also fail. On Default Light and High Contrast Light, phase 29 measured that the reset floors allow a smallest session-vs-reset dE of 14.7 and 15.3, not 20, so this item covers light themes too.
- **Target:** `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/themes/`, `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/summary/accounts_gradient_tests.rs`.
- **First step:** list what else can separate sessions: fewer fixed inks on a session row, or a second visual channel that the style guide allows. Include the two Astra role inks (phase 21) in the assessment. Send the showrunner the options with shots before any palette code.
- **Completion condition:** in all four themes, every session ink has CIEDE2000 >= 15 from each fixed ink that can share its row, every pair of session inks has CIEDE2000 >= 20, and covered contrast is >= 4.5:1 with phase 18's unrounded calculation. In both light themes, session-vs-reset CIEDE2000 is >= 20 within 45° of hue. Root i keeps entry i of the list, and each tile title keeps its summary row's ink. Until completion, retain the phase 26 session palette limits and the light session and reset limits that phases 29 and 30 hold.
