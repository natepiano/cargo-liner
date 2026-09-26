//! Every cargo-tile module and its `#[cfg(test)]` tests, compiled as an integration test.
//!
//! Cargo builds the `cargo-tile` binary before any integration test and passes its path as
//! `CARGO_BIN_EXE_cargo-tile`, so tests that execute the binary run the one built from these
//! sources. `[[bin]] test = false` in `Cargo.toml` and `#![cfg(not(test))]` in `src/main.rs`
//! keep the binary target from compiling these tests a second time. The list mirrors the `mod`
//! declarations in `src/main.rs`, plus the test-only `shim_registration`; a module left out fails
//! to compile wherever a listed module names it.

#[path = "../src/app.rs"]
mod app;
#[path = "../src/birth_stamp/mod.rs"]
mod birth_stamp;
#[path = "../src/capture.rs"]
mod capture;
#[path = "../src/census/mod.rs"]
mod census;
#[path = "../src/cli.rs"]
mod cli;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/constants.rs"]
mod constants;
#[path = "../src/globals.rs"]
mod globals;
#[path = "../src/hook.rs"]
mod hook;
#[path = "../src/interaction.rs"]
mod interaction;
#[path = "../src/keymap.rs"]
mod keymap;
#[path = "../src/probe.rs"]
mod probe;
#[path = "../src/progress/mod.rs"]
mod progress;
#[path = "../src/registration.rs"]
mod registration;
#[path = "../src/render.rs"]
mod render;
#[path = "../src/root_scan/mod.rs"]
mod root_scan;
#[path = "../src/roster.rs"]
mod roster;
#[path = "../src/sccache.rs"]
mod sccache;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/shim_registration/mod.rs"]
mod shim_registration;
#[path = "../src/terminal.rs"]
mod terminal;
#[path = "../src/theme/mod.rs"]
mod theme;
#[path = "../src/tiles.rs"]
mod tiles;
#[path = "../src/wrap.rs"]
mod wrap;
