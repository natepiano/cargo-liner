//! Codex prompt labels share the drawn name cut policy.

#![expect(clippy::expect_used, reason = "nonempty fixture prompt has a label")]
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::*;

#[test]
fn codex_prompt_labels_trim_the_space_at_the_drawn_cut() {
    let prompt = format!("{} more work", "x".repeat(CODEX_PROMPT_LABEL_MAX - 2));
    let thread = CodexThread {
        cwd:          PathBuf::from("/fixture"),
        created_ms:   0,
        name:         None,
        first_prompt: prompt,
        rollout:      PathBuf::from("/fixture/rollout.jsonl"),
    };
    let label = thread.label().expect("prompt label");
    let area = Rect::new(0, 0, 100, 1);
    let mut buffer = Buffer::empty(area);
    Paragraph::new(label).render(area, &mut buffer);
    let drawn: String = (0..area.width).map(|x| buffer[(x, 0)].symbol()).collect();
    let before = drawn.split(TRUNCATION_MARK).next().expect("cut prefix");
    assert!(drawn.contains(TRUNCATION_MARK));
    assert!(!before.ends_with(char::is_whitespace), "{drawn:?}");
}
