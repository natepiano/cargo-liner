mod dispatch;
mod editor_terminal;

pub(super) use dispatch::*;
pub(super) use editor_terminal::open_finder;
pub(super) use editor_terminal::open_in_editor;
pub(super) use editor_terminal::open_paths_in_editor;
pub(super) use editor_terminal::open_terminal;
#[cfg(test)]
pub(super) use tui_pane::set_last_mouse_pos_for_test;
