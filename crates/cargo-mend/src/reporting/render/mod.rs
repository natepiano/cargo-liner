mod color;
mod diagnostic;
mod human;
mod summary;
mod timing;
mod types;

pub(crate) use human::render_human_report;
pub(crate) use timing::render_timing;
pub(crate) use types::ColorMode;
pub(crate) use types::CompilerStats;
pub(crate) use types::OutputFormat;
