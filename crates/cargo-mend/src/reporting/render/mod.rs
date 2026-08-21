mod color;
mod diagnostic;
mod human;
mod output;
mod summary;
mod timing;

pub(crate) use human::render_human_report;
pub(crate) use output::ColorMode;
pub(crate) use output::CompilerStats;
pub(crate) use output::OutputFormat;
pub(crate) use timing::render_timing;
