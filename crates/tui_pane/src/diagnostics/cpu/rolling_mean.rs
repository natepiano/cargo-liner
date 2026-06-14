use super::CPU_SMOOTHING_WINDOW_POLLS;
use super::VecDeque;

/// Bounded rolling-mean window for utilization samples.
///
/// Damps single-poll spikes and the transient zeros the macOS GPU
/// counter publishes mid-update. Used for the GPU row here and for the
/// per-process CPU column in consumers' process lists.
#[derive(Clone, Debug, Default)]
pub struct RollingMean {
    window: VecDeque<f32>,
}

impl RollingMean {
    /// Fold `sample` into the window and return the new mean. The first
    /// sample is the mean of one — no zero dilution.
    pub fn push(&mut self, sample: f32) -> f32 {
        self.window.push_back(sample);
        if self.window.len() > CPU_SMOOTHING_WINDOW_POLLS {
            self.window.pop_front();
        }
        let len = u16::try_from(self.window.len()).unwrap_or(u16::MAX);
        self.window.iter().sum::<f32>() / f32::from(len)
    }
}
