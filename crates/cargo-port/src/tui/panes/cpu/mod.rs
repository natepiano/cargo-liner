mod pane;
mod render;

pub use pane::CpuPane;
pub use render::cpu_required_pane_height;
use render::render_cpu_pane_body;

pub use super::constants::CPU_PANE_WIDTH;
