//! Render-ready text carried by response envelopes.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Whether an envelope supplies render-ready output blocks.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "envelope_presentation")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum EnvelopePresentation {
    /// The constructor has no presentation contract for this response.
    NotProvided,
    /// The engine explicitly rendered every block the front end should show.
    RenderedBlocks {
        /// Ordered, self-contained blocks for the front end to publish.
        blocks: Vec<RenderedOutputBlock>,
    },
}

impl EnvelopePresentation {
    /// State that the engine considered this response and found nothing to show.
    pub(crate) const fn nothing_to_show() -> Self { Self::RenderedBlocks { blocks: Vec::new() } }

    /// Replace the current presentation with one rendered block.
    pub(crate) fn replace_with(&mut self, rendered_output_block: RenderedOutputBlock) {
        *self = rendered_output_block.into();
    }
}

impl From<RenderedOutputBlock> for EnvelopePresentation {
    fn from(rendered_output_block: RenderedOutputBlock) -> Self {
        Self::RenderedBlocks {
            blocks: vec![rendered_output_block],
        }
    }
}

/// One complete user-facing summary and its supporting detail.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "rendered_output_block")]
pub(crate) struct RenderedOutputBlock {
    /// The short front-end heading for this response.
    pub(crate) summary: String,
    /// The complete explanation or recovery instruction shown below the heading.
    pub(crate) detail:  String,
}

/// Render a response whose engine message is already the complete detail.
pub(crate) fn engine_message_block(summary: &str, message: &str) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: summary.to_owned(),
        detail:  message.to_owned(),
    }
}

/// Render an ambiguity refusal with the candidate reservations kept visible.
pub(crate) fn ambiguous_first_touch_block(
    message: &str,
    candidate_reservation_ids: &[String],
) -> RenderedOutputBlock {
    let candidates = candidate_reservation_ids
        .iter()
        .map(|reservation_id| format!("- `{reservation_id}`"))
        .collect::<Vec<_>>()
        .join("\n");
    RenderedOutputBlock {
        summary: "cargo-berth could not select one active reservation for this edit.".to_owned(),
        detail:  format!("{message}\n\nReservations named by this response:\n\n{candidates}"),
    }
}

/// Render the recovery action selected by coordination-identity validation.
pub(crate) fn coordination_identity_block(
    summary: &str,
    rejection_kind: &str,
    recovery_action: &str,
) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: summary.to_owned(),
        detail:  format!(
            "COORDINATION IDENTITY: {rejection_kind} requires one recovery action before continuing: {recovery_action}"
        ),
    }
}

/// Render invalid retained history with its typed subject and recovery command.
pub(crate) fn replay_failure_block(
    summary: &str,
    reason: &str,
    subject: &str,
) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: summary.to_owned(),
        detail:  format!(
            "REPLAY HARD STOP: {reason} for {subject}. Review the cargo-berth journal. If the retained order is invalid and may be discarded, run `cargo-berth init --reinitialize-after-review --json`."
        ),
    }
}

/// Render a successful automatic reservation widening.
pub(crate) fn automatic_widening_block(
    reservation_id: &str,
    added_scopes: &[String],
) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: "cargo-berth widened this worktree reservation footprint.".to_owned(),
        detail:  format!(
            "AUTO-WIDEN: reservation {reservation_id} now covers {}",
            added_scopes.join(", ")
        ),
    }
}

/// The exact board actions for one reservation's outstanding incursions.
pub(crate) struct IncursionResolutionGuidance<'action> {
    /// How many unresolved incidents remain for the straying reservation.
    pub(crate) outstanding_count: usize,
    /// Resolve only the incident named by the surrounding block.
    pub(crate) incident_action:   &'action str,
    /// Resolve every outstanding incident for the straying reservation.
    pub(crate) every_action:      &'action str,
}

/// Render one drift incursion that the current board still lists as outstanding.
pub(crate) fn outstanding_incursion_block(
    reservation_id: &str,
    entered_paths: &[String],
    foreign_reservation_ids: &[String],
    incident_id: &str,
    commit_context: &str,
) -> String {
    format!(
        "INCURSION: reservation {reservation_id} entered {}, held by {}; incident {incident_id}.{commit_context} STOP. Resolve with `cargo-berth resolve {reservation_id} --incursion {incident_id}` before making more changes.",
        entered_paths.join(", "),
        foreign_reservation_ids.join(", ")
    )
}

/// Render one board incursion with its current single-incident and all-incident actions.
pub(crate) fn outstanding_board_incursion_block(
    reservation_id: &str,
    entered_paths: &[String],
    foreign_reservation_ids: &[String],
    incident_id: &str,
    resolution: &IncursionResolutionGuidance<'_>,
) -> String {
    let incident = format!(
        "INCURSION: reservation {reservation_id} entered {}, held by {}; incident {incident_id}.",
        entered_paths.join(", "),
        foreign_reservation_ids.join(", ")
    );
    let outstanding_incursions = match resolution.outstanding_count {
        1 => "1 outstanding incursion".to_owned(),
        outstanding_count => format!("{outstanding_count} outstanding incursions"),
    };
    format!(
        "{incident} STOP. Reservation {reservation_id} has {outstanding_incursions}. Resolve this incident with `cargo-berth {}` or every outstanding incursion with `cargo-berth {}` before making more changes.",
        resolution.incident_action, resolution.every_action
    )
}

/// Render one bypass marker recovered and filed during the current board read.
pub(crate) fn recovered_bypass_block(marker_name: &str) -> String {
    format!(
        "Recovered bypass marker {marker_name}: a bypass recorded earlier while the journal was unwritable has now been filed in the journal."
    )
}

/// Render the complete decision document for a blocked edit.
pub(crate) fn blocked_edit_refusal_block(detail: &str) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary:
            "cargo-berth refused this edit because another reservation holds the requested paths."
                .to_owned(),
        detail:  detail.to_owned(),
    }
}

/// Render guidance for a successful edit whose session mapping is not reusable.
pub(crate) fn degraded_session_mapping_block(detail: &str) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary:
            "cargo-berth protected this edit, but later commands cannot reuse its session mapping."
                .to_owned(),
        detail:  detail.to_owned(),
    }
}

/// Render the refusal used when a live board cannot establish incursion membership.
pub(crate) fn unverifiable_incursion_block() -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: "cargo-berth could not verify the live incursion state.".to_owned(),
        detail:  "STOP: the PostToolUse drift response named an incursion, but a current board read could not confirm whether it still needs resolution."
            .to_owned(),
    }
}

/// Render a released reservation whose integration evidence no longer proves its work is in trunk.
pub(crate) fn lost_integration_evidence_block(detail: &str) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: "cargo-berth detected lost integration evidence for released work.".to_owned(),
        detail:  detail.to_owned(),
    }
}

/// Render every actionable board notice with the count computed from the rendered entries.
pub(crate) fn actionable_board_notices_block(details: &[String]) -> RenderedOutputBlock {
    RenderedOutputBlock {
        summary: format!(
            "cargo-berth found {} actionable coordination notice(s).",
            details.len()
        ),
        detail:  details.join("\n"),
    }
}

/// Render an orphaned outstanding reservation and its evidence-supported recovery commands.
pub(crate) fn orphaned_outstanding_block(
    reservation_id: &str,
    protected_tip: &str,
    recoverability: &str,
    recovery_commands: &[String],
) -> String {
    let commands = recovery_commands
        .iter()
        .map(|command| format!("`cargo-berth {command}`"))
        .collect::<Vec<_>>()
        .join(" or ");
    format!(
        "ORPHANED OUTSTANDING: reservation {reservation_id} at protected tip {protected_tip} is {recoverability}. Answer it with {commands} after reviewing the work."
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::EnvelopePresentation;
    use super::actionable_board_notices_block;
    use super::ambiguous_first_touch_block;
    use super::automatic_widening_block;
    use super::coordination_identity_block;
    use super::engine_message_block;
    use super::lost_integration_evidence_block;
    use super::orphaned_outstanding_block;
    use super::outstanding_incursion_block;
    use super::replay_failure_block;
    use super::unverifiable_incursion_block;

    const FRONT_END_CORPUS: &str = include_str!("../tests/fixtures/front_end_corpus.json");

    #[test]
    fn explicit_nothing_is_distinct_from_missing_presentation() {
        assert_ne!(
            EnvelopePresentation::nothing_to_show(),
            EnvelopePresentation::NotProvided
        );
    }

    #[test]
    fn engine_message_block_keeps_the_engine_words() {
        let block = engine_message_block("Engine heading.", "Engine detail.");
        assert_eq!(block.summary, "Engine heading.");
        assert_eq!(block.detail, "Engine detail.");
    }

    #[test]
    fn ambiguous_first_touch_block_names_every_candidate() {
        let block = ambiguous_first_touch_block(
            "Choose one reservation.",
            &["reservation-a".to_owned(), "reservation-b".to_owned()],
        );
        assert!(block.detail.contains("- `reservation-a`"));
        assert!(block.detail.contains("- `reservation-b`"));
    }

    #[test]
    fn coordination_identity_block_carries_the_recovery_action() {
        let block = coordination_identity_block(
            "Identity rejected.",
            "stale_session_mapping",
            "`cargo-berth identity clear-session --json`",
        );
        assert_eq!(
            block.detail,
            "COORDINATION IDENTITY: stale_session_mapping requires one recovery action before continuing: `cargo-berth identity clear-session --json`"
        );
    }

    #[test]
    fn replay_failure_block_names_the_subject_and_command() {
        let block = replay_failure_block(
            "Invalid history.",
            "unknown_reservation",
            "reservation reservation-a",
        );
        assert!(
            block
                .detail
                .contains("unknown_reservation for reservation reservation-a")
        );
        assert!(
            block
                .detail
                .contains("cargo-berth init --reinitialize-after-review --json")
        );
    }

    #[test]
    fn automatic_widening_block_names_the_reservation_and_scopes() {
        let block = automatic_widening_block(
            "reservation-a",
            &["file:first.rs".to_owned(), "file:second.rs".to_owned()],
        );
        assert_eq!(
            block.detail,
            "AUTO-WIDEN: reservation reservation-a now covers file:first.rs, file:second.rs"
        );
    }

    #[test]
    fn outstanding_incursion_block_names_the_board_confirmed_incursion() {
        let block = outstanding_incursion_block(
            "reservation-a",
            &["foreign.rs".to_owned()],
            &["reservation-b".to_owned()],
            "incident-a",
            "",
        );
        assert!(block.contains("reservation reservation-a entered foreign.rs"));
        assert!(block.contains("incident incident-a"));
    }

    #[test]
    fn unverifiable_incursion_block_requires_an_immediate_stop() {
        let block = unverifiable_incursion_block();
        assert!(block.detail.starts_with("STOP:"));
    }

    #[test]
    fn lost_integration_evidence_block_preserves_the_recovery_command() {
        let block = lost_integration_evidence_block(
            "Run `cargo-berth resolve reservation-a --integrated-as trunk-a`.",
        );
        assert!(block.detail.contains("--integrated-as trunk-a"));
    }

    #[test]
    fn actionable_board_notices_block_counts_its_details() {
        let block = actionable_board_notices_block(&[
            "First notice.".to_owned(),
            "Second notice.".to_owned(),
        ]);
        assert_eq!(
            block.summary,
            "cargo-berth found 2 actionable coordination notice(s)."
        );
        assert_eq!(block.detail, "First notice.\nSecond notice.");
    }

    #[test]
    fn orphaned_outstanding_block_carries_every_recovery_command() {
        let block = orphaned_outstanding_block(
            "reservation-a",
            "protected-tip-a",
            "commit_unavailable",
            &[
                "resolve reservation-a --retire-orphan --why <reason>".to_owned(),
                "resolve reservation-a --abandon --why <reason>".to_owned(),
            ],
        );
        assert!(block.contains("--retire-orphan --why <reason>"));
        assert!(block.contains("--abandon --why <reason>"));
    }

    #[test]
    fn corpus_contains_the_fifty_independent_rendering_cases() -> Result<(), String> {
        let corpus = parsed_front_end_corpus()?;
        let entries = corpus["entries"]
            .as_array()
            .ok_or_else(|| "front-end corpus should contain entries".to_owned())?;
        let unfamiliar_fallbacks = entries
            .iter()
            .filter(|entry| {
                entry["expected"]["stdout"]
                    .as_str()
                    .is_some_and(|stdout| stdout.contains("does not yet render"))
            })
            .count();
        assert_eq!(entries.len(), 50);
        assert_eq!(unfamiliar_fallbacks, 3);
        Ok(())
    }

    #[test]
    fn corpus_generic_message_matches_the_named_engine_block() -> Result<(), String> {
        let block = engine_message_block(
            "cargo-berth could not establish coordination state after Bash.",
            "An outcome this installation has never been told about.",
        );
        assert_corpus_stdout_block(
            "test_every_hook_route_states_the_engines_words_for_an_unfamiliar_response#7",
            &block,
        )
    }

    #[test]
    fn corpus_widening_matches_the_named_engine_block() -> Result<(), String> {
        let block =
            automatic_widening_block("reservation-widened", &["file:widened.rs".to_owned()]);
        assert_corpus_stdout_block("test_incursion_board_read_cost_is_constant#4", &block)
    }

    #[test]
    fn corpus_outstanding_incursion_matches_the_named_engine_block() -> Result<(), String> {
        let detail = outstanding_incursion_block(
            "reservation-straying",
            &["path-0.rs".to_owned()],
            &["foreign-0".to_owned()],
            "incident-0",
            "",
        );
        let block = engine_message_block(
            "cargo-berth detected drift that requires an immediate stop.",
            &detail,
        );
        assert_corpus_stdout_block("test_outstanding_incursion_emits_stop_text", &block)
    }

    #[test]
    fn corpus_unverifiable_incursion_matches_the_named_engine_block() -> Result<(), String> {
        let block = unverifiable_incursion_block();
        assert_corpus_stdout_block("test_invalid_live_board_fails_closed", &block)
    }

    #[test]
    fn corpus_ambiguity_matches_the_named_engine_block() -> Result<(), String> {
        let corpus = parsed_front_end_corpus()?;
        let entry = corpus_entry(
            &corpus,
            "test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message",
        )?;
        let message = entry["engine_responses"]["check"]["body"]["message"]
            .as_str()
            .ok_or_else(|| "ambiguity corpus response should carry a message".to_owned())?;
        let candidates = vec![
            "01a054d8-6797-7ec2-8907-07a71169d947".to_owned(),
            "01a054d8-67f0-7d73-b6af-1c5016b1e9ef".to_owned(),
        ];
        let block = ambiguous_first_touch_block(message, &candidates);
        let expected_stderr = entry["expected"]["stderr"]
            .as_str()
            .ok_or_else(|| "ambiguity corpus entry should carry expected stderr".to_owned())?;
        assert_eq!(expected_stderr.trim_end_matches('\n'), block.detail);
        Ok(())
    }

    #[test]
    fn corpus_coordination_recovery_matches_the_named_engine_block() -> Result<(), String> {
        let block = coordination_identity_block(
            "cargo-berth rejected drift under the current coordination identity.",
            "stale_session_mapping",
            "`cd '{FIXTURE_ROOT}/repository' && 'cargo-berth' 'identity' 'clear-session' '--json'`",
        );
        assert_corpus_stdout_block(
            "test_hooks_render_coordination_identity_recovery_actions_without_message#2",
            &block,
        )
    }

    #[test]
    fn corpus_replay_failure_matches_the_named_engine_block() -> Result<(), String> {
        let block = replay_failure_block(
            "cargo-berth stopped on invalid reservation history after Bash.",
            "unknown_reservation",
            "reservation 01991f4d-77d8-7f5f-9a1f-000000000001",
        );
        assert_corpus_stdout_block(
            "test_typed_replay_failure_routes_without_message_in_every_consumer#2",
            &block,
        )
    }

    #[test]
    fn corpus_lost_evidence_matches_the_named_engine_block() -> Result<(), String> {
        let entry_name = "test_hooks_render_both_lost_evidence_recoveries";
        let detail = corpus_stdout_context(entry_name)?;
        let block = lost_integration_evidence_block(&detail);
        assert_corpus_stdout_block(entry_name, &block)
    }

    #[test]
    fn corpus_orphan_recovery_matches_the_named_engine_blocks() -> Result<(), String> {
        let first = orphaned_outstanding_block(
            "01a057cf-550b-7e81-a469-a0dc171c82af",
            "c13950fb8e8eb52e6ecdd9e0e2d70da8e358ab1d",
            "recoverable_from_branch",
            &["resolve 01a057cf-550b-7e81-a469-a0dc171c82af --recovered".to_owned()],
        );
        let second = orphaned_outstanding_block(
            "01a057cf-576a-79b2-b292-c29fd9d77620",
            "812d1b215ec9a351334cff53148a69e4ee4a0fb2",
            "recoverable_from_branch",
            &["resolve 01a057cf-576a-79b2-b292-c29fd9d77620 --recovered".to_owned()],
        );
        let block = actionable_board_notices_block(&[first, second]);
        assert_corpus_stdout_block(
            "test_session_start_renders_real_orphan_recovery_actions",
            &block,
        )
    }

    fn assert_corpus_stdout_block(
        entry_name: &str,
        block: &super::RenderedOutputBlock,
    ) -> Result<(), String> {
        let corpus = parsed_front_end_corpus()?;
        let entry = corpus_entry(&corpus, entry_name)?;
        let stdout = entry["expected"]["stdout"]
            .as_str()
            .ok_or_else(|| format!("{entry_name} should carry expected stdout"))?;
        let rendered = serde_json::from_str::<Value>(stdout)
            .map_err(|error| format!("{entry_name} stdout should be JSON: {error}"))?;
        assert_eq!(
            rendered["systemMessage"].as_str(),
            Some(block.summary.as_str())
        );
        assert_eq!(
            rendered["hookSpecificOutput"]["additionalContext"].as_str(),
            Some(block.detail.as_str())
        );
        Ok(())
    }

    fn corpus_stdout_context(entry_name: &str) -> Result<String, String> {
        let corpus = parsed_front_end_corpus()?;
        let entry = corpus_entry(&corpus, entry_name)?;
        let stdout = entry["expected"]["stdout"]
            .as_str()
            .ok_or_else(|| format!("{entry_name} should carry expected stdout"))?;
        let rendered = serde_json::from_str::<Value>(stdout)
            .map_err(|error| format!("{entry_name} stdout should be JSON: {error}"))?;
        rendered["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{entry_name} should carry additional context"))
    }

    fn parsed_front_end_corpus() -> Result<Value, String> {
        serde_json::from_str(FRONT_END_CORPUS)
            .map_err(|error| format!("front-end corpus should parse: {error}"))
    }

    fn corpus_entry<'corpus>(
        corpus: &'corpus Value,
        entry_name: &str,
    ) -> Result<&'corpus Value, String> {
        corpus["entries"]
            .as_array()
            .ok_or_else(|| "front-end corpus should contain entries".to_owned())?
            .iter()
            .find(|entry| entry["name"].as_str() == Some(entry_name))
            .ok_or_else(|| format!("front-end corpus should contain {entry_name}"))
    }
}
