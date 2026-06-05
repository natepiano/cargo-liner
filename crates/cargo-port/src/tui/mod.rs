mod app;
mod background;
mod columns;
mod constants;
mod finder;
mod input;
mod integration;
mod interaction;
mod keymap;
mod keymap_ui;
mod overlays;
mod pane;
mod panes;
mod project_list;
mod render;
mod running_targets;
mod sccache;
mod settings;
mod state;
mod terminal;
#[cfg(test)]
mod test_support;
mod theme_roles;

pub use terminal::run;
