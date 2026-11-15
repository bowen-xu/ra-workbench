use super::*;

#[test]
fn migrated_history_projects_shared_terminal_outcomes() {
    assert_eq!(shared_outcome_label(TurnOutcome::Completed), "completed");
    assert_eq!(
        shared_outcome_label(TurnOutcome::Interrupted),
        "interrupted"
    );
    assert_eq!(
        crate::agent::repository::legacy_ai_runs::status_for_outcome(
            TurnOutcome::Completed,
            true,
            false,
        ),
        Some("needs_review")
    );
    assert_eq!(
        crate::agent::repository::legacy_ai_runs::status_for_outcome(
            TurnOutcome::Completed,
            false,
            false,
        ),
        None
    );
    assert_eq!(
        crate::agent::repository::legacy_ai_runs::status_for_outcome(
            TurnOutcome::Interrupted,
            false,
            false,
        ),
        Some("partial")
    );
}
