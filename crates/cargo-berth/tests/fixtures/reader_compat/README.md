# Reader compatibility fixture

`journal.ndjson` is frozen, untouched output of an earlier `cargo-berth` binary,
the one built from the tree that added this fixture in commit `5c210b8e` (crate
version `0.1.0-dev`): one claim followed by the first reconciliation's
`merge_extent_observed` record. It is kept unchanged to prove that the current
reader decodes records an earlier binary wrote; for example, its
`identity_inputs` omit `claude_code_session_id`.
Its SHA-256 is `ba5ecbffdadcc18880231ef2213d11acc7909657262200b29f338a2f8afd7427`.
No record is synthesized by the test.

The fixture was generated in a temporary repository using these steps:

1. Initialize a Git repository on `main` and commit `README.md` containing
   `reader compatibility fixture\n`.
2. Run the freshly built `cargo-berth init --json`, then commit its
   `.claude/config/berth.toml`.
3. Write `uncommitted fixture work\n` to untracked `claimed.txt`.
4. Run `cargo-berth claim file:claimed.txt --why 'reader compatibility fixture' --json`.
5. Run `cargo-berth board --json` to append the merge observation.
6. Copy the journal and the repository, worktree, and run identity files here;
   save the Git objects with `git bundle create repository.bundle --all`.

The bundle preserves the exact commit objects named by the records. It contains
only the two fixture commits, with no work from the surrounding workspace.
The original temporary repository was removed after generation.

Each test clones that bundle into its own temporary directory and copies the
ledger bytes and identities unchanged. It recreates `claimed.txt` and renews
activity with the freshly built reader, so the fixture does not become stale
after 24 hours. The test checks that the original journal remains an exact byte
prefix. Reconciliation discovers the relocated worktree through its identity.
The reader selected for the actual check must still decode the original record.

The JSON files freeze the command status and complete payload, or the complete
hook response. Comparisons restate the temporary worktree path, journal byte
offsets, and renewed activity timestamps. All other fields remain asserted,
including journal generations and both extents. SessionStart must state
nothing, because the fixture's reservation is on no integration order and
raises no notice; PostToolUse sees a newly written `additional.txt` and must
publish the frozen auto-widen notice. Neither hook can pass by returning zero
or remaining silent after a ledger error, which both hooks state.

Run `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth reader_compat`.
Set `CARGO_BERTH_EXECUTABLE` to an absolute executable path to inspect an
installed reader; unset it to use the newly built binary. A relative override
is resolved before entering the temporary repository.
