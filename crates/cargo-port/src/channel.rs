//! Crate-wide channel seam.
//!
//! Re-exports `crossbeam-channel` under stable names so the render loop
//! can `Select` over heterogeneous receivers (input, background, CI
//! fetch, clean, example, CPU samples) and block until one is ready —
//! see `crate::tui::terminal`.
//!
//! Every channel that reaches the render-loop `Select` flows through
//! this module: swap a site's `use std::sync::mpsc::X` for
//! `use crate::channel::X` (the type names are identical) and
//! `mpsc::channel()` for [`unbounded`].
//!
//! Std-only internal channels that never reach the loop deliberately
//! stay on `std::sync::mpsc` (imported as `StdSender`/`StdReceiver`
//! alongside this module where they coexist): the lint supervisor and
//! trigger channels in `src/lint/runtime.rs`, and the watcher's notify /
//! disk-done / git-done channels in `src/watcher/`. Keeping them on std
//! marks, at the type level, that they are not `Select` sources.

pub(crate) use crossbeam_channel::Receiver;
pub(crate) use crossbeam_channel::Select;
pub(crate) use crossbeam_channel::SendError;
pub(crate) use crossbeam_channel::Sender;
pub(crate) use crossbeam_channel::TryRecvError;
pub(crate) use crossbeam_channel::unbounded;
