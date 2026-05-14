mod app;
mod background;
mod columns;
mod constants;
mod cpu;
mod finder;
mod input;
mod integration;
mod interaction;
mod keymap_ui;
mod overlays;
mod pane;
mod panes;
mod project_list;
mod render;
mod settings;
mod state;
mod terminal;
#[cfg(test)]
mod test_support;

pub use terminal::run;
