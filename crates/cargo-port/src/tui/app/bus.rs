//! Cross-cutting event bus skeleton (Phase 18 — smoke test).
//!
//! `Event` is what subsystems react to: a single trigger that may
//! fan out to several subscribers. `Command` is a single targeted
//! action with a known caller and a known effect — used when a
//! handler needs to defer work that would otherwise force a
//! `&mut Background`-style borrow conflict.
//!
//! Phase 18 wires up the queue-and-drain skeleton and routes one
//! event end-to-end (`Event::ServiceSignal`). Subscribers take
//! `&mut App` for now (Phase 17 lesson 4 — defer `HandlerCtx`
//! typed parameters until a borrow conflict forces them). The
//! `EventHandler` / `HandlerCtx` types are scaffolding: Phase 19's
//! `apply_lint_config_change` + `apply_config` bundle is the
//! actual borrow-checker stress test that will exercise them.
//!
//! Drain order: events drain first; each event handler may dispatch
//! commands or publish further events. Then commands drain. The
//! outer loop alternates until both queues are empty so commands
//! can publish new events and vice versa.

use std::collections::VecDeque;

use crate::http::ServiceKind;
use crate::http::ServiceSignal;

#[derive(Clone, Copy, Debug)]
pub(super) enum Event {
    ServiceSignal(ServiceSignal),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Command {
    SpawnServiceRetry(ServiceKind),
}

#[derive(Default)]
pub(super) struct EventBus {
    events:   VecDeque<Event>,
    commands: VecDeque<Command>,
}

impl EventBus {
    pub(super) fn new() -> Self { Self::default() }

    pub(super) fn publish(&mut self, ev: Event) { self.events.push_back(ev); }

    pub(super) fn dispatch(&mut self, cmd: Command) { self.commands.push_back(cmd); }

    pub(super) fn pop_event(&mut self) -> Option<Event> { self.events.pop_front() }

    pub(super) fn pop_command(&mut self) -> Option<Command> { self.commands.pop_front() }
}

/// Borrowed view passed to subscribers that take typed subsystem
/// borrows instead of `&mut App`. Phase 18 doesn't use this — its
/// reactor takes `&mut self` on App and reaches the bus via
/// `self.bus.dispatch(...)` directly. Phase 19 is the upgrade
/// site if a five-subsystem borrow conflict forces typed
/// parameters.
#[allow(
    dead_code,
    reason = "Phase 19 scaffolding — exercised when apply_lint_config_change + apply_config land on the bus"
)]
pub(super) struct HandlerCtx<'a> {
    pub(super) commands: &'a mut VecDeque<Command>,
}

#[allow(
    dead_code,
    reason = "Phase 19 scaffolding — exercised when apply_lint_config_change + apply_config land on the bus"
)]
impl HandlerCtx<'_> {
    pub(super) fn dispatch(&mut self, cmd: Command) { self.commands.push_back(cmd); }
}

#[allow(
    dead_code,
    reason = "Phase 19 scaffolding — exercised when apply_lint_config_change + apply_config land on the bus"
)]
pub(super) trait EventHandler {
    fn handle(&mut self, ev: Event, ctx: &mut HandlerCtx<'_>);
}
