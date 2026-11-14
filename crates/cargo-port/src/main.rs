//! `cargo-port` — a TUI for inspecting and managing Rust projects.

mod cache_paths;
mod channel;
mod ci;
mod config;
mod constants;
mod enrichment;
mod http;
mod lint;
mod process_observation;
mod project;
mod scan;
mod sccache;
mod support;
#[cfg(test)]
mod test_support;
mod themes;
mod tui;
mod watcher;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    limit_allocator_arenas();
    tui::run()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(
    unsafe_code,
    reason = "GNU libc arena policy runs before worker threads"
)]
fn limit_allocator_arenas() {
    // SAFETY: This GNU libc call takes integer options. It runs before cargo-port starts worker
    // threads.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, constants::GNU_MALLOC_ARENA_LIMIT) };
}
