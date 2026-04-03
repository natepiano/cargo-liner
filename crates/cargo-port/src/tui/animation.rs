use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cycle {
    period: Duration,
}

impl Cycle {
    pub const fn new(period: Duration) -> Self {
        assert!(
            period.as_secs() > 0 || period.subsec_nanos() > 0,
            "animation cycle period must be non-zero"
        );
        Self { period }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCycle {
    frames: &'static [&'static str],
    cycle:  Cycle,
}

impl FrameCycle {
    pub const fn new(frames: &'static [&'static str], period: Duration) -> Self {
        assert!(
            !frames.is_empty(),
            "frame cycle requires at least one frame"
        );
        Self {
            frames,
            cycle: Cycle::new(period),
        }
    }

    pub fn frame_at(self, elapsed: Duration) -> &'static str {
        let frame_count = u128::try_from(self.frames.len()).unwrap_or(u128::MAX);
        let period = self.cycle.period.as_nanos();
        let elapsed = elapsed.as_nanos() % period;
        let frame_index = elapsed.saturating_mul(frame_count) / period;
        let frame_index = usize::try_from(frame_index).unwrap_or(self.frames.len() - 1);
        self.frames[frame_index]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Static(&'static str),
    Animated(FrameCycle),
}

impl Icon {
    pub fn frame_at(self, elapsed: Duration) -> &'static str {
        match self {
            Self::Static(icon) => icon,
            Self::Animated(cycle) => cycle.frame_at(elapsed),
        }
    }
}

pub const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const BRAILLE_SPINNER: FrameCycle =
    FrameCycle::new(BRAILLE_FRAMES, Duration::from_millis(1000));
pub const LINT_SPINNER_FRAMES: &[&str] = &[
    "⠉⠉", "⠈⠙", "⠀⠹", "⠀⢸", "⠀⣰", "⢀⣠", "⣀⣀", "⣄⡀", "⣆⠀", "⡇⠀", "⠏⠀", "⠋⠁",
];
pub const LINT_SPINNER: FrameCycle =
    FrameCycle::new(LINT_SPINNER_FRAMES, Duration::from_millis(1200));
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FrameCycle;

    const TEST_FRAMES: &[&str] = &["a", "b", "c", "d"];
    const TEST_FRAME_CYCLE: FrameCycle = FrameCycle::new(TEST_FRAMES, Duration::from_millis(400));

    #[test]
    fn frame_cycle_returns_first_frame_at_zero() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::ZERO), "a");
    }

    #[test]
    fn frame_cycle_advances_after_each_interval() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(100)), "b");
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(200)), "c");
    }

    #[test]
    fn frame_cycle_wraps_after_full_period() {
        assert_eq!(TEST_FRAME_CYCLE.frame_at(Duration::from_millis(400)), "a");
    }
}
