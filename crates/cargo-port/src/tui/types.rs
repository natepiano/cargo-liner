/// A bounded cursor for scrollable lists. Replaces raw `usize` index + manual
/// bounds checking with a single type that enforces invariants.
#[derive(Default, Clone)]
pub struct ScrollState {
    pos: usize,
}

impl ScrollState {
    pub const fn pos(&self) -> usize { self.pos }

    pub fn set(&mut self, pos: usize) { self.pos = pos; }

    pub fn up(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    pub fn down(&mut self, len: usize) {
        if len > 0 && self.pos < len - 1 {
            self.pos += 1;
        }
    }

    pub fn to_top(&mut self) { self.pos = 0; }

    pub fn to_bottom(&mut self, len: usize) { self.pos = len.saturating_sub(1); }

    /// Clamp position to `0..len`. Useful after the backing list shrinks.
    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.pos = 0;
        } else if self.pos >= len {
            self.pos = len - 1;
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum FocusTarget {
    #[default]
    ProjectList,
    DetailFields,
    CiRuns,
    ScanLog,
}
