mod app;
mod background;
mod columns;
mod constants;
mod dismiss_target;
mod finder;
mod hit_test;
mod input;
mod integration;
mod interaction;
mod keymap;
mod keymap_ui;
mod messages;
mod overlays;
mod panes;
mod project_list;
mod render;
mod render_context;
mod running_targets;
mod sccache;
mod settings;
mod state;
mod terminal;
#[cfg(test)]
mod test_support;
mod theme_roles;

pub use terminal::run;
