# cargo-tile design decisions

Decisions the user made about how cargo-tile works. A phase that changes one of
these needs the user's approval first.

## Running commands: summary and cell

The summary shows each top-level command; its own cell shows the detailed
version. Main does this correctly today. The ports must keep it exactly as main
has it. No phase may remove either. (User, 2026-10-29.)
