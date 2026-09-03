//! Independent oracle over the frozen front-end corpus.
//!
//! `tests/fixtures/front_end_corpus.json` records what the three installed hooks printed
//! for real engine responses. Its whole value is that it is independent of the code it
//! checks, so nothing here regenerates it, relaxes a comparison, or drops an entry
//! because nothing drives it any more. This suite compares one entry's frozen text
//! against the live binary and holds every entry to a coverage partition: each is driven
//! by a named test or carries the measured reason it cannot be. The partition is an
//! identity over whatever the fixture carries, so a floor on the fixture's own size
//! carries the rest of that promise: a deletion cannot balance itself out to green.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

const CARGO_BERTH_SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
/// The one corpus entry this suite compares against the real binary itself.
const AMBIGUOUS_FIRST_TOUCH_ENTRY: &str =
    "test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message";
/// Corpus entries whose frozen text this suite compares, each named beside the test
/// that drives it.
const THIS_SUITE_TEXT_COMPARED_ENTRIES: [(&str, &str); 1] = [(
    AMBIGUOUS_FIRST_TOUCH_ENTRY,
    "the_ambiguous_first_touch_presentation_matches_the_frozen_corpus_text",
)];
/// Corpus entries whose frozen text `tests/hooks.rs` compares against the real
/// binary, each named beside the test that drives it.
const ACCEPTANCE_TEXT_COMPARED_ENTRIES: [(&str, &str); 22] = [
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message",
        "session_identity_recoveries_preserve_the_frozen_corpus_text",
    ),
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message#2",
        "post_tool_use_states_its_coordination_identity_recovery",
    ),
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message#3",
        "stale_marker_recovery_preserves_the_frozen_corpus_text",
    ),
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message#5",
        "session_identity_recoveries_preserve_the_frozen_corpus_text",
    ),
    (
        "test_typed_replay_failure_routes_without_message_in_every_consumer",
        "replay_failure_emits_a_fail_open_object",
    ),
    (
        "test_typed_replay_failure_routes_without_message_in_every_consumer#2",
        "post_tool_use_states_the_replay_failure_route",
    ),
    (
        "test_typed_replay_failure_routes_without_message_in_every_consumer#3",
        "session_start_states_the_replay_failure_route",
    ),
    (
        "test_a_named_widening_with_nothing_to_report_still_says_nothing",
        "post_tool_use_with_nothing_to_report_emits_nothing",
    ),
    (
        "test_incursion_board_read_cost_is_constant",
        "post_tool_use_states_the_incursion_resolve_instruction",
    ),
    (
        "test_incursion_board_read_cost_is_constant#2",
        "post_tool_use_states_every_incursion_of_one_bash_call",
    ),
    (
        "test_incursion_board_read_cost_is_constant#3",
        "post_tool_use_with_nothing_to_report_emits_nothing",
    ),
    (
        "test_incursion_board_read_cost_is_constant#4",
        "post_tool_use_states_the_auto_widen_notice",
    ),
    (
        "test_outstanding_incursion_emits_stop_text",
        "post_tool_use_states_the_incursion_resolve_instruction",
    ),
    (
        "test_recorded_incursion_emits_no_stop_text",
        "post_tool_use_stops_repeating_an_answered_incursion",
    ),
    (
        "test_recorded_incursion_preserves_concurrent_widening_feedback",
        "post_tool_use_states_a_widening_after_the_incursion_was_answered",
    ),
    (
        "test_recorded_incursion_preserves_lost_evidence_feedback",
        "post_tool_use_states_lost_evidence_after_the_incursion_was_answered",
    ),
    (
        "test_hooks_render_both_lost_evidence_recoveries",
        "post_tool_use_states_the_rewritten_trunk_evidence_recovery",
    ),
    (
        "test_hooks_render_both_lost_evidence_recoveries#2",
        "session_start_states_the_rewritten_trunk_evidence_recovery",
    ),
    (
        "test_hooks_render_both_lost_evidence_recoveries#3",
        "post_tool_use_states_the_unresolvable_trunk_evidence_recovery",
    ),
    (
        "test_hooks_render_both_lost_evidence_recoveries#4",
        "session_start_states_the_unresolvable_trunk_evidence_recovery",
    ),
    (
        "test_session_start_renders_real_orphan_recovery_actions#2",
        "session_start_states_the_unavailable_orphan_recovery_actions",
    ),
    (
        "test_session_start_renders_real_orphan_recovery_actions",
        "session_start_publishes_the_engine_board_report",
    ),
];
/// Every corpus entry no test drives, and what in the engine accounts for it.
///
/// The frozen corpus records what the three installed hooks printed while a shell front
/// end read the engine's JSON and decided from it. That front end is retired for three
/// wrappers that exec the binary, so the corpus holds entries whose text nothing can
/// produce any more. Each gets a row stating what in the engine makes it unproducible,
/// measured against the code rather than dated with a phase number, so a reader meets a
/// decided list rather than a residue. `every_corpus_entry_is_text_compared_or_named_unproven`
/// holds the list to the corpus: a row for an entry a test now drives fails, a row for an
/// entry the fixture does not carry fails, and an entry this list forgets fails too.
const CORPUS_ENTRIES_WITHOUT_A_TEST: [UnprovenCorpusEntry; 27] = [
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_hooks_render_coordination_identity_recovery_actions_without_message#4",
        because: "drift sweeps this worktree's coordination run marker before validating it, on \
                  the same predicate the marker validation rejects on, so a post-Bash process \
                  never presents a stale marker; the pre-edit route reaches it because check \
                  runs no such preflight",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_a_nested_tag_no_table_names_still_reaches_the_advisory_route",
        because: "the frozen heading is the retired shell's fallback for a status absent from its \
                  installed table, which no engine constant states, and the payload needs a \
                  widening tag this binary's enum does not carry",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#3",
        because: "board serializes its status from its own enum, so no board response carries a \
                  status this installation cannot name, nor the frozen message that goes with it",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#4",
        because: "the frozen heading names coordination state, which the retired shell keyed on \
                  exit 4; board now answers an unreadable ledger in its own words at SessionStart",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#5",
        because: "the frozen heading names reaching the ledger, which the retired shell keyed on \
                  exit 6; board now answers an exhausted lock deadline in its own words",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#6",
        because: "the frozen response is a terminal-view failure reaching the reader, and session \
                  start reads the board as JSON, which opens no terminal to fail",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#7",
        because: "the post-Bash twin of the coordination-state heading: drift answers an \
                  unreadable ledger in its own words after Bash",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#8",
        because: "the heading matches, but drift's rejected-selection detail always appends the \
                  command to rerun by hand, which the frozen detail does not carry",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#9",
        because: "the post-Bash twin of the reach-the-ledger heading: drift answers an exhausted \
                  lock deadline in its own words after Bash",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_hooks_render_coordination_identity_recovery_actions_without_message#6",
        because: "the frozen first action reruns a check command line, and post-tool-use supplies \
                  its own drift command as the original command of every rejection it reports",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_hooks_render_coordination_identity_recovery_actions_without_message#8",
        because: "the frozen single-action rendering happens only for an original command holding \
                  an argument that is not text, and post-tool-use supplies three text arguments",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_incursion_in_both_board_sections_fails_closed",
        because: "the board partitions one incident list by a two-variant status, so no board \
                  response places one incident in both the outstanding and the answered section",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_post_bash_reports_an_unnamed_drift_status_in_the_engine_words",
        because: "the frozen heading is the retired shell's fallback for a status absent from its \
                  installed table, which no engine constant states",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_post_bash_reports_an_unnamed_drift_status_in_the_engine_words#2",
        because: "the same retired fallback heading, reached in the corpus through a terminal-view \
                  exit this binary never returns from a drift comparison",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_invalid_live_board_fails_closed",
        because: "the retired shell front end parsed the board's JSON from a separate process, so a \
                  malformed board body was a real failure it could report; post-tool-use now calls \
                  board in process and receives a typed envelope, and that envelope's exit is \
                  always clear, so no board this engine builds can be unreadable",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response",
        because: "the frozen allow is the retired shell's fail-open branch for exit 4, taken over \
                  an envelope whose status and payload kind are both \
                  a_status_no_installed_table_names; check serializes each from its own enum, so \
                  no run of this binary emits either name",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#2",
        because: "the same synthetic status reached through exit 5, where the retired shell \
                  blocked on the exit code alone; pre-tool-use decides from a typed check \
                  response rather than an exit code, and still emits no such status",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_hooks_render_coordination_identity_recovery_actions_without_message#7",
        because: "the pre-edit twin of the single-action rendering: check_recovery_command_line builds \
                  all five arguments from Rust text, so runnable_arguments never answers \
                  RecoveryCommandContainsNonTextArgument and a session_worktree_mismatch reached \
                  through this hook always renders both recovery actions",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_allows_an_unfamiliar_clear_response",
        because: "the frozen envelope clears under a_clear_status_no_installed_table_names, and \
                  check's clear statuses are variants of its own enum, so no response arrives \
                  carrying a clear status this installation cannot name",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_blocks_an_unnamed_status_on_its_exit_code",
        because: "the frozen block is the retired shell's exit-1 branch over the same synthetic \
                  status; blocking is now a typed decision inside pre-tool-use, and the status it \
                  would have to block on is one no build emits",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_blocks_an_unnamed_status_on_its_exit_code#2",
        because: "the exit-2 twin of the same synthetic status; two exit codes reaching one branch \
                  was a fact about the shell's installed table, and the engine consults no table",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_still_refuses_output_that_cannot_speak_for_itself",
        because: "the frozen refusal is the retired shell reading a claim envelope back from a \
                  check invocation; pre-tool-use holds the typed check response in process, so no \
                  verb can disagree with the request that produced it",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_still_refuses_output_that_cannot_speak_for_itself#2",
        because: "the same refusal over a clear envelope carrying exit 1; pre-tool-use reads the \
                  check response as a typed value and renders the PreToolUse protocol object \
                  itself, so no envelope is serialized for a reader to find a status and an exit \
                  code disagreeing in",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_pre_edit_still_refuses_output_that_cannot_speak_for_itself#3",
        because: "the same refusal over an envelope stating exit 3 while the process exited 1; a \
                  hook verb renders no envelope and owns its exit through \
                  CommandOutputOwnership::HookRendered, so the pre-edit route has neither of the \
                  two numbers this entry needs to disagree",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_unreadable_generated_validator_names_broken_installation",
        because: "the frozen sentence names berth_pre_edit-missing-envelope-validation.jq, a \
                  generated validator no installation carries any more; no engine constant states \
                  it, and the wrapper's binary-absent refusal that replaces it is asserted in \
                  tests/test_hook_rendering.py, outside this crate",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_unreadable_generated_validator_names_broken_installation#2",
        because: "the post-Bash twin, naming berth_post_bash-missing-envelope-validation.jq and \
                  the repair notice the shell printed for it; the wrapper's binary-absent notice \
                  replaces it and is asserted outside this crate",
    },
    UnprovenCorpusEntry::UnproducibleByThisEngine {
        name:    "test_unreadable_generated_validator_names_broken_installation#3",
        because: "the SessionStart twin, naming berth_session_start-missing-envelope-validation.jq \
                  and its reconciliation repair notice; same retired artifact, same replacement \
                  asserted outside this crate",
    },
];
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const FRONT_END_CORPUS_JSON: &str = include_str!("fixtures/front_end_corpus.json");
/// The floor the frozen corpus may never fall below.
///
/// A ratchet, not a total: it rises when the fixture grows and has no legitimate reason
/// to fall, so lowering it is never incidental to other work. The coverage partition is
/// an identity over whatever the fixture carries, which leaves it balanced when an entry
/// and the row claiming it are deleted together. This is the assertion that the fixture
/// itself has not shrunk, and it lives here rather than in the fixture so that dropping
/// an entry cannot be made to look like a passing suite from inside the file being cut.
const MINIMUM_FROZEN_CORPUS_ENTRIES: usize = 50;
/// The suites whose test names this file's coverage tables cite.
///
/// A cited test placed in a suite this list does not name gets no guard, so widening the
/// list is how a new suite joins the tables.
const CITED_SUITES: [(&str, &str); 2] = [
    ("tests/hooks.rs", include_str!("hooks.rs")),
    (
        "tests/front_end_corpus.rs",
        include_str!("front_end_corpus.rs"),
    ),
];
const SCRATCH_ROOT: &str = "/tmp/claude";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

type ShellOracleResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct RenderedOutputBlockEvidence<'a> {
    summary: &'a str,
    detail:  &'a str,
}

/// Why one corpus entry has no test driving it.
///
/// One answer, and it is a closed one: an entry this engine cannot produce is not work
/// anybody can finish, and the acceptance gate cannot ask for it. Nothing may be moved
/// here because reaching it is laborious — an entry a repository state does produce is
/// open work, and it earns a second variant on the day one is found, not a row here.
enum UnprovenCorpusEntry {
    /// No real `cargo-berth` binary can produce this entry's frozen text.
    UnproducibleByThisEngine {
        /// The corpus entry this row accounts for.
        name:    &'static str,
        /// What in the engine makes the frozen text unproducible.
        because: &'static str,
    },
}

impl UnprovenCorpusEntry {
    const fn name(&self) -> &'static str {
        match self {
            Self::UnproducibleByThisEngine { name, .. } => name,
        }
    }

    const fn account(&self) -> &'static str {
        match self {
            Self::UnproducibleByThisEngine { because, .. } => because,
        }
    }
}

/// Every corpus entry is either driven by a test or carries the reason it has none.
///
/// The gate is an identity rather than a total. Each entry the fixture froze is claimed by
/// exactly one of three sets — the entry this suite text-compares itself, the entries
/// `tests/hooks.rs` text-compares, and the entries `CORPUS_ENTRIES_WITHOUT_A_TEST`
/// accounts for — and every count comes from the fixture on the run, never from a
/// constant. That distinction is the whole point. A gate phrased over a number is
/// satisfiable by lowering the number, so an entry that quietly loses its test reads as a
/// passing suite. Here an entry no set claims fails, an entry two sets claim fails, and a
/// row naming an entry the fixture does not carry fails.
#[test]
fn every_corpus_entry_is_text_compared_or_named_unproven() -> ShellOracleResult<()> {
    let entry_names = corpus_entry_names()?;
    for name in &entry_names {
        require_one_coverage_claim(name)?;
    }
    for (entry_name, _) in ACCEPTANCE_TEXT_COMPARED_ENTRIES
        .iter()
        .chain(THIS_SUITE_TEXT_COMPARED_ENTRIES.iter())
    {
        require_corpus_entry_exists(&entry_names, entry_name, "booked as text-compared")?;
    }
    for entry in &CORPUS_ENTRIES_WITHOUT_A_TEST {
        require_corpus_entry_exists(&entry_names, entry.name(), "named as having no test")?;
        if entry.account().is_empty() {
            return Err(failure(format!(
                "{} should state why it has no test",
                entry.name()
            )));
        }
    }
    Ok(())
}

/// The frozen corpus never shrinks.
///
/// The partition next door proves every entry the fixture carries is claimed exactly
/// once, which says nothing about how many entries it carries: delete an entry together
/// with the row that claimed it and the identity still holds. An entry records what a
/// real hook printed for a real engine response, so losing one loses evidence that
/// cannot be re-derived from the code under test. Raise this floor when the corpus
/// grows; a deletion that needs it lowered is the finding this test exists to make loud.
#[test]
fn the_frozen_corpus_never_shrinks() -> ShellOracleResult<()> {
    let carried = corpus_entry_names()?.len();
    if carried < MINIMUM_FROZEN_CORPUS_ENTRIES {
        return Err(failure(format!(
            "the front-end corpus carries {carried} entries but may never fall below \
             {MINIMUM_FROZEN_CORPUS_ENTRIES}; an entry records what a real hook printed and \
             is not re-derivable, so restore it rather than lowering the floor"
        )));
    }
    Ok(())
}

/// The frozen text oracle: one corpus entry compared against what the real binary emits.
///
/// The fixture records renderings taken from real engine output, and its value is that it
/// is independent of the code it checks. A difference here is a finding about the engine,
/// never a reason to refresh the fixture.
#[test]
fn the_ambiguous_first_touch_presentation_matches_the_frozen_corpus_text() -> ShellOracleResult<()>
{
    let real_envelope = ambiguous_first_touch_envelope()?;
    let rendered_output_blocks = rendered_output_blocks(&real_envelope)?;
    let entry = corpus_entry(AMBIGUOUS_FIRST_TOUCH_ENTRY)?;
    compare_ambiguous_first_touch_text(&entry, &real_envelope, &rendered_output_blocks)
}

fn corpus_entries() -> ShellOracleResult<Vec<Value>> {
    let corpus: Value = serde_json::from_str(FRONT_END_CORPUS_JSON)?;
    corpus
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| failure("front-end corpus should carry an entries array"))
}

fn corpus_entry_names() -> ShellOracleResult<Vec<String>> {
    corpus_entries()?
        .iter()
        .map(|entry| {
            entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| failure("every front-end corpus entry should carry a name"))
        })
        .collect()
}

fn corpus_entry(name: &str) -> ShellOracleResult<Value> {
    corpus_entries()?
        .into_iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| failure(format!("front-end corpus should carry the entry {name}")))
}

/// Require exactly one of the three coverage sets to claim one corpus entry.
fn require_one_coverage_claim(name: &str) -> ShellOracleResult<()> {
    let claimants = [
        (
            "this suite",
            THIS_SUITE_TEXT_COMPARED_ENTRIES
                .iter()
                .any(|(entry_name, _)| *entry_name == name),
        ),
        (
            "tests/hooks.rs",
            ACCEPTANCE_TEXT_COMPARED_ENTRIES
                .iter()
                .any(|(entry_name, _)| *entry_name == name),
        ),
        (
            "CORPUS_ENTRIES_WITHOUT_A_TEST",
            CORPUS_ENTRIES_WITHOUT_A_TEST
                .iter()
                .any(|entry| entry.name() == name),
        ),
    ]
    .into_iter()
    .filter_map(|(claimant, claims)| claims.then_some(claimant))
    .collect::<Vec<_>>();
    match claimants.as_slice() {
        [] => Err(failure(format!(
            "{name} has no test comparing its frozen text and no row in \
             CORPUS_ENTRIES_WITHOUT_A_TEST saying why"
        ))),
        [_] => Ok(()),
        _ => Err(failure(format!(
            "{name} is claimed by more than one coverage set: {}",
            claimants.join(", ")
        ))),
    }
}

fn require_corpus_entry_exists(
    entry_names: &[String],
    name: &str,
    booking: &str,
) -> ShellOracleResult<()> {
    if entry_names.iter().any(|entry_name| entry_name == name) {
        return Ok(());
    }
    Err(failure(format!(
        "{name} is {booking} but the front-end corpus carries no such entry"
    )))
}

fn compare_ambiguous_first_touch_text(
    entry: &Value,
    real_envelope: &Value,
    rendered_output_blocks: &[RenderedOutputBlockEvidence<'_>],
) -> ShellOracleResult<()> {
    let [block] = rendered_output_blocks else {
        return Err(failure(format!(
            "real ambiguous first-touch response should render exactly one block, found {}",
            rendered_output_blocks.len()
        )));
    };
    if block.summary.is_empty() {
        return Err(failure(
            "real ambiguous first-touch block should carry a summary of its own",
        ));
    }
    let expected = entry
        .get("expected")
        .and_then(|expected| expected.get("stderr"))
        .and_then(Value::as_str)
        .ok_or_else(|| failure("ambiguous first-touch corpus entry should carry stderr"))?
        .trim_end_matches('\n');
    let normalized_detail = normalize_reservation_ids(entry, real_envelope, block.detail)?;
    if normalized_detail != expected {
        return Err(failure(format!(
            "ambiguous first-touch production presentation differs from the corpus:\nexpected={expected:?}\nactual={normalized_detail:?}"
        )));
    }
    Ok(())
}

fn normalize_reservation_ids(
    entry: &Value,
    real_envelope: &Value,
    rendered_detail: &str,
) -> ShellOracleResult<String> {
    let actual = real_envelope
        .get("reservations")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("real ambiguous response should carry reservations"))?;
    let expected = entry
        .get("engine_responses")
        .and_then(|responses| responses.get("check"))
        .and_then(|response| response.get("body"))
        .and_then(|body| body.get("reservations"))
        .and_then(Value::as_array)
        .ok_or_else(|| failure("ambiguous corpus response should carry reservations"))?;
    if actual.len() != expected.len() {
        return Err(failure(format!(
            "ambiguous reservation count differs: real={}, corpus={}",
            actual.len(),
            expected.len()
        )));
    }
    actual.iter().zip(expected).try_fold(
        rendered_detail.to_owned(),
        |detail, (actual, expected)| {
            let actual = actual
                .as_str()
                .ok_or_else(|| failure("real ambiguous reservation id should be a string"))?;
            let expected = expected
                .as_str()
                .ok_or_else(|| failure("corpus ambiguous reservation id should be a string"))?;
            Ok(detail.replace(actual, expected))
        },
    )
}

fn ambiguous_first_touch_envelope() -> ShellOracleResult<Value> {
    fs::create_dir_all(SCRATCH_ROOT)?;
    let repository = TempDir::new_in(SCRATCH_ROOT)?;
    run_git(repository.path(), &["init", "-b", "main"])?;
    run_git(
        repository.path(),
        &["config", "user.email", "oracle@example.invalid"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.name", "Front End Oracle"],
    )?;
    fs::write(
        repository.path().join("README.md"),
        "front-end shell oracle\n",
    )?;
    run_git(repository.path(), &["add", "README.md"])?;
    run_git(
        repository.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
    )?;

    let initialized = run_berth(repository.path(), &["init", "--json"], "shell-init")?;
    require_success(&initialized, "scratch cargo-berth init")?;
    let older_claim = run_berth(
        repository.path(),
        &["claim", "tree:shared", "--run", FIRST_RUN, "--json"],
        "shell-selection",
    )?;
    require_success(&older_claim, "older overlapping claim")?;
    let newer_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:shared/child.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        "shell-selection",
    )?;
    require_success(&newer_claim, "newer overlapping claim")?;
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))?;

    let ambiguous = run_berth(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        "shell-selection",
    )?;
    if ambiguous.status.code() != Some(1) {
        return Err(failure(format!(
            "ambiguous first touch should exit 1: stdout={} stderr={}",
            String::from_utf8_lossy(&ambiguous.stdout),
            String::from_utf8_lossy(&ambiguous.stderr)
        )));
    }
    Ok(serde_json::from_slice(&ambiguous.stdout)?)
}

fn run_git(repository: &Path, arguments: &[&str]) -> ShellOracleResult<()> {
    let output = support::git_command()
        .args(arguments)
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(failure(format!(
            "git {} failed in scratch repository: stdout={} stderr={}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn run_berth(repository: &Path, arguments: &[&str], session_id: &str) -> ShellOracleResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository)
        .env(CARGO_BERTH_SESSION_ENVIRONMENT, session_id)
        .output()?)
}

fn require_success(output: &Output, operation: &str) -> ShellOracleResult<()> {
    if !output.status.success() {
        return Err(failure(format!(
            "{operation} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn rendered_output_blocks(
    envelope: &Value,
) -> ShellOracleResult<Vec<RenderedOutputBlockEvidence<'_>>> {
    let envelope = required_object(envelope, "real cargo-berth envelope")?;
    let presentation = envelope
        .get("presentation")
        .ok_or_else(|| failure("real cargo-berth envelope should carry presentation"))?;
    let presentation = required_object(presentation, "real cargo-berth presentation")?;
    let kind = required_string_member(presentation, "kind", "real cargo-berth presentation")?;
    if kind != "rendered_blocks" {
        return Err(failure(format!(
            "real ambiguous first-touch presentation should use rendered_blocks, found {kind:?}"
        )));
    }
    let blocks = presentation
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("real rendered_blocks presentation should carry an array"))?;
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| rendered_output_block(block, index))
        .collect()
}

fn rendered_output_block(
    block: &Value,
    index: usize,
) -> ShellOracleResult<RenderedOutputBlockEvidence<'_>> {
    let context = format!("real cargo-berth presentation block {index}");
    let block = required_object(block, context.as_str())?;
    let summary = required_string_member(block, "summary", context.as_str())?;
    let detail = required_string_member(block, "detail", context.as_str())?;
    Ok(RenderedOutputBlockEvidence { summary, detail })
}

fn required_object<'a>(
    value: &'a Value,
    context: &str,
) -> ShellOracleResult<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| failure(format!("{context} should be an object")))
}

fn required_string_member<'a>(
    object: &'a serde_json::Map<String, Value>,
    member: &str,
    context: &str,
) -> ShellOracleResult<&'a str> {
    object
        .get(member)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} should carry string member {member}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

/// Every test this file names as covering a corpus entry has to exist in a cited suite.
///
/// The coverage tables pair a corpus entry with the test that drives it, but the entry half
/// is the only half the partition checks: a test could be deleted and its row left behind,
/// and the partition would still balance while the entry was covered by nothing. This reads
/// the cited suites and requires each name to be defined in one of them, so a deleted test
/// fails the gate instead of leaving a coverage claim standing on its own.
#[test]
fn every_cited_acceptance_test_exists_in_the_suite() -> ShellOracleResult<()> {
    for (entry_name, test_name) in ACCEPTANCE_TEXT_COMPARED_ENTRIES
        .iter()
        .chain(THIS_SUITE_TEXT_COMPARED_ENTRIES.iter())
    {
        let definition = format!("fn {test_name}(");
        if !CITED_SUITES
            .iter()
            .any(|(_, suite)| suite.contains(definition.as_str()))
        {
            return Err(failure(format!(
                "{entry_name} is booked as text-compared by {test_name}, which none of {} defines",
                CITED_SUITES
                    .iter()
                    .map(|(path, _)| *path)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(())
}
