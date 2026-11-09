use crossterm::style::Colored;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// The terminal colour policy selected at app startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorMode {
    /// Draw with the configured colours.
    Colored,
    /// Draw without terminal colours.
    NoColor,
}

impl ColorMode {
    /// Clear frame colours without changing text or modifiers when colours are disabled.
    pub fn clear_colors(self, buffer: &mut Buffer) {
        if self == Self::Colored {
            return;
        }
        for cell in &mut buffer.content {
            cell.fg = Color::Reset;
            cell.bg = Color::Reset;
        }
    }

    /// Read the terminal colour policy once at app startup.
    #[must_use]
    pub fn from_environment() -> Self {
        if Colored::ansi_color_disabled_memoized() {
            Self::NoColor
        } else {
            Self::Colored
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::Command;

    use ratatui::Terminal;
    use ratatui::TerminalOptions;
    use ratatui::Viewport;
    use ratatui::backend::CrosstermBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;
    use ratatui::widgets::Paragraph;

    use super::ColorMode;

    const CHILD_ENV: &str = "TUI_PANE_NO_COLOR_MODIFIERS_CHILD";

    #[test]
    fn colored_frame_keeps_colors_text_and_modifiers() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(
            0,
            0,
            "text",
            Style::new()
                .fg(Color::Red)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        );
        let original = buffer.clone();
        ColorMode::Colored.clear_colors(&mut buffer);
        assert_eq!(buffer, original);
    }

    #[test]
    fn no_color_backend_keeps_bold_and_reverse_before_text() -> io::Result<()> {
        if std::env::var_os(CHILD_ENV).is_none() {
            let module = module_path!()
                .split_once("::")
                .map_or(module_path!(), |(_, module)| module);
            let test = format!("{module}::no_color_backend_keeps_bold_and_reverse_before_text");
            let output = Command::new(std::env::current_exe()?)
                .args(["--exact", &test, "--nocapture"])
                .env(CHILD_ENV, "1")
                .env("NO_COLOR", "1")
                .output()?;
            assert!(
                output.status.success(),
                "NO_COLOR child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "the child must run the byte test"
            );
            return Ok(());
        }
        let mut bytes = Vec::new();
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(&mut bytes),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 8, 2)),
            },
        )?;
        terminal.draw(|frame| {
            frame.render_widget(
                Paragraph::new("bold").style(
                    Style::new()
                        .fg(Color::Red)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(0, 0, 8, 1),
            );
            frame.render_widget(
                Paragraph::new("reverse").style(
                    Style::new()
                        .fg(Color::Green)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                ),
                Rect::new(0, 1, 8, 1),
            );
            ColorMode::from_environment().clear_colors(frame.buffer_mut());
        })?;
        drop(terminal);
        let output = String::from_utf8_lossy(&bytes);
        assert!(
            output.contains("\u{1b}[1mbold"),
            "bold must reach its text without a reset: {output:?}"
        );
        assert!(
            output.contains("\u{1b}[7mreverse"),
            "reverse must reach its text without a reset: {output:?}"
        );
        assert!(
            !output.contains("\u{1b}[;m"),
            "colour suppression must not reset modifiers: {output:?}"
        );
        for color_code in ["[38;", "[48;", "[31m", "[32m", "[43m", "[44m"] {
            assert!(
                !output.contains(color_code),
                "NO_COLOR must emit no colour code: {output:?}"
            );
        }
        Ok(())
    }
}
